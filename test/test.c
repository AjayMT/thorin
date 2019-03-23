
#include <stdint.h>


void func()
{
  int64_t var = 12;
  if (var) {
    uintptr_t b = 2;
    ++b;
  }
}

int main(int argc, char *argv[])
{
  float pi = 3.14;
  func();
  return 0;
}
