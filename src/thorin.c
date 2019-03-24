
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <mach/mach.h>
#include <mach/mach_vm.h>
#include <mach/thread_state.h>

#include "mig/mach_exc.h"


typedef void (*exc_callback)(void*, uintptr_t);
static exc_callback global_cb;
static mach_port_t global_task;
static mach_port_t global_task_exc;
static void *global_scope;


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
  uintptr_t rbp = ((x86_thread_state64_t *)old_state)->__rbp;
  global_cb(global_scope, rbp);

  return KERN_FAILURE;
}


void setup(pid_t child, exc_callback cb, void *scope)
{
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
  global_cb = cb;
  global_scope = scope;

  size_t req_size = sizeof(union __RequestUnion__mach_exc_subsystem);
  size_t rep_size = sizeof(union __ReplyUnion__mach_exc_subsystem);
  mach_msg_server_once(
    mach_exc_server,
    req_size > rep_size ? req_size : rep_size,
    task_exception_port,
    0
    );
}


void read_addr(void *buffer, uintptr_t address, size_t size)
{
  kern_return_t kret;
  mach_vm_size_t local_size = size;
  kret = mach_vm_read_overwrite(global_task, address, (mach_vm_size_t)size, (mach_vm_address_t)buffer, &local_size);
  if (kret != KERN_SUCCESS) {
    printf("mach_vm_read failed: %s\n", mach_error_string(kret));
    return;
  }
}
