
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <spawn.h>

#ifdef __APPLE__

#include <mach/mach.h>
#include <mach/mach_vm.h>
#include <mach/thread_state.h>
#include "mig/mach_exc.h"

#elif __linux__

#include <sys/types.h>
#include <sys/wait.h>
#include <sys/ptrace.h>
#include <sys/user.h>
#include <linux/ptrace.h>
#include <elf.h>
#include <signal.h>
#include <errno.h>
#include <string.h>
#include <unistd.h>

struct iovec {
  void *iov_base;
  unsigned int iov_len;
};

#endif

typedef void (*exc_callback)(void*, void*, uintptr_t, uintptr_t);
static exc_callback global_cb;
static void *global_scope;
static void *global_types;


#ifdef __APPLE__

static mach_port_t global_task;
static mach_port_t global_task_exc;

extern boolean_t mach_exc_server (mach_msg_header_t *msg, mach_msg_header_t *reply);

kern_return_t catch_mach_exception_raise_state (
  mach_port_t exception_port,
  exception_type_t exception,
  const mach_exception_data_t code,
  mach_msg_type_number_t codeCnt,
  int *flavor,
  const thread_state_t old_state,
  mach_msg_type_number_t old_stateCnt,
  thread_state_t new_state,
  mach_msg_type_number_t *new_stateCnt
  )
{ return KERN_FAILURE; }

kern_return_t catch_mach_exception_raise (
  mach_port_t exception_port,
  mach_port_t thread,
  mach_port_t task,
  exception_type_t exception,
  mach_exception_data_t code,
  mach_msg_type_number_t codeCnt
  )
{ return KERN_FAILURE; }

kern_return_t catch_mach_exception_raise_state_identity (
  mach_port_t exception_port,
  mach_port_t thread,
  mach_port_t task,
  exception_type_t exception,
  mach_exception_data_t code,
  mach_msg_type_number_t codeCnt,
  int *flavor,
  thread_state_t old_state,
  mach_msg_type_number_t old_stateCnt,
  thread_state_t new_state,
  mach_msg_type_number_t *new_stateCnt
  )
{
  x86_thread_state64_t state = *(x86_thread_state64_t *)old_state;

  global_cb(global_scope, global_types, state.__rbp, state.__rip);

  return KERN_FAILURE;
}

#elif __linux__
static pid_t global_child = 0;

void perform_callback(pid_t child)
{
  struct user_regs_struct regs;
  memset(&regs, 0, sizeof(regs));
  struct iovec iov;
  iov.iov_base = &regs;
  iov.iov_len = sizeof(regs);

  long r = ptrace(PTRACE_GETREGSET, child, NT_PRSTATUS, &iov);
  if (r == -1) {
    printf("PTRACE_GETREGSET failed: %s\n", strerror(errno));
    return;
  }

  global_cb(global_scope, global_types, regs.rbp, regs.rip);
}

void setup_inferior(const char *target, char *argv[])
{
  ptrace(PTRACE_TRACEME, 0, NULL, NULL);
  execv(target, argv);
}

void attach_to_inferior(pid_t child) {
  while(1) {
    int status;
    waitpid(child, &status, 0);

    if (WIFSTOPPED(status) && WSTOPSIG(status) == SIGTRAP) {
      ptrace(PTRACE_CONT, child, NULL, NULL);
    } else if (WIFEXITED(status)) {
      printf("Child process exited\n");
      return;
    } else {
      global_child = child;
      perform_callback(child);
      return;
    }
  }
}
#endif

void setup(const char *target, exc_callback cb, void *scope, void *types)
{
  global_cb = cb;
  global_scope = scope;
  global_types = types;

  pid_t child = 0;

#ifdef __APPLE__
  posix_spawnattr_t attr;
  posix_spawnattr_init(&attr);
  posix_spawnattr_setflags(&attr, 0x100); // disable ASLR on MacOS
  posix_spawnp(&child, target, NULL, &attr, NULL, NULL);

  mach_port_t task;
  mach_port_t task_exception_port;
  kern_return_t kret;

  kret = task_for_pid(mach_task_self(), child, &task);
  if (kret != KERN_SUCCESS) {
    printf("task_for_pid failed: %s\n", mach_error_string(kret));
    return;
  }

  kret = mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &task_exception_port);
  if (kret != KERN_SUCCESS) {
    printf("mach_port_allocate failed: %s\n", mach_error_string(kret));
    return;
  }

  kret = mach_port_insert_right(
    mach_task_self(),
    task_exception_port,
    task_exception_port,
    MACH_MSG_TYPE_MAKE_SEND
    );
  if (kret != KERN_SUCCESS) {
    printf("mach_port_insert_right failed: %s\n", mach_error_string(kret));
    return;
  }

  kret = task_set_exception_ports(
    task,
    EXC_MASK_ALL,
    task_exception_port,
    EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES,
    x86_THREAD_STATE64
    );
  if (kret != KERN_SUCCESS) {
    printf("task_set_exception_ports failed: %s\n", mach_error_string(kret));
    return;
  }

  global_task = task;
  global_task_exc = task_exception_port;

  size_t req_size = sizeof(union __RequestUnion__mach_exc_subsystem);
  size_t rep_size = sizeof(union __ReplyUnion__mach_exc_subsystem);
  mach_msg_server_once(
    mach_exc_server,
    req_size > rep_size ? req_size : rep_size,
    task_exception_port,
    0
    );

#elif __linux__
  do {
    child = fork();
    switch (child) {
    case 0:
      setup_inferior(target, NULL);
      break;
    case -1:
      break;
    default:
      attach_to_inferior(child);
      break;
    }
  } while (child == -1 && errno == EAGAIN);
#endif
}


void read_addr(void *buffer, uintptr_t address, size_t size)
{
#ifdef __APPLE__
  kern_return_t kret;
  mach_vm_size_t local_size = size;
  kret = mach_vm_read_overwrite(global_task, address, (mach_vm_size_t)size, (mach_vm_address_t)buffer, &local_size);
  if (kret != KERN_SUCCESS) {
    printf("mach_vm_read failed: %s\n", mach_error_string(kret));
    memset(buffer, 0, size);
    return;
  }
#elif __linux__

  // stolen shamelessly from https://github.com/scanmem/scanmem/blob/master/ptrace.c

#ifdef __GNUC__
#define UNLIKELY(x)   __builtin_expect(!!(x), 0)
#else
#define UNLIKELY(x)   (x)
#endif

  size_t nread = 0;
  errno = 0;
  for (nread = 0; nread < size; nread += sizeof(long)) {
    const char *ptrace_address = (char *)address + nread;
    long ptraced_long = ptrace(PTRACE_PEEKDATA, global_child, ptrace_address, NULL);

    if (UNLIKELY(ptraced_long == -1L && errno != 0)) {
      if (errno == EIO || errno == EFAULT) {
        int j;
        for (j = 1, errno = 0; j < sizeof(long); j++, errno = 0) {
          ptraced_long = ptrace(PTRACE_PEEKDATA, global_child, ptrace_address - j, NULL);
          if ((ptraced_long == -1L) && (errno == EIO || errno == EFAULT))
            continue;

          uint8_t* new_memory_ptr = (uint8_t*)(&ptraced_long) + j;
          memcpy(buffer + nread, new_memory_ptr, sizeof(long) - j);
          nread += sizeof(long) - j;
          break;
        }
      }

      break;
    }

    memcpy(buffer + nread, &ptraced_long, sizeof(long));
  }
#endif
}
