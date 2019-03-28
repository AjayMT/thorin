
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>


struct my_type {
  uint64_t val;
  char baz;
};


void func()
{
  int64_t var = 12;
  if (var) {
    uintptr_t b = 2;
    ++b;
  }

  uint64_t foo = 11;

  if (foo) {
    double pi = 22 / 7;
    printf("z\n");
  }
}

int main(int argc, char *argv[])
{
  float pi = 3.14;
  func();
  struct my_type my_obj = { .val = 42, .baz = 'F' };
  float *ppi = &pi;

  printf("hello\n");

  int *n = NULL;
  *n = 12;

  return 0;
}
