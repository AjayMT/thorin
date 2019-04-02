
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>


struct point2 {
  int64_t x;
  int64_t y;
};


struct my_type {
  uint64_t val;
  char baz;
  struct point2 loc;
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
    double pi = 22.0 / 7.0;
    double *ptr = &pi;
  }
}

int main(int argc, char *argv[])
{
  float pi = 3.14;
  func();
  struct my_type my_obj = { .val = 42, .baz = 'F', .loc = { .x = 12, .y = 13 } };
  float *ppi = &pi;
  char *str = "hello";
  uint64_t num = 42;

  __builtin_trap();

  return 0;
}
