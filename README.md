
# thorin -- the DWARF debugger
`thorin` is a simple debugger for linux and MacOS, like `gdb`'s (very) little brother. It currently only works on C programs and x86_64 architectures.

`thorin` is still in development -- see the TODO section.

## Installation
Get [Rust](http://rust-lang.org) and
```
cargo install thorin
```

## Usage
```
thorin <target-program>
```

`thorin` will start the target program and wait for an exception, like a segfault.

If the program exits successfully, nothing happens.

If the program triggers an exception, thorin will suspend it and capture its state. You can inspect the program's state through the thorin console:
```
thorin> help
Commands:
  (print|show|get) <variable-name>:  Print the value of a variable.
  read <address> <count> <type>:     Read the value at <address>. <type>
                                     is the type of the value, <count> is the
                                     number of values to read.
  help:                              Print this help message.
  (exit|quit):                       Quit thorin.
```

In order to access variables and other source-level information, thorin needs to be able to read debugging information in the program's object file. To provide this information, all you need to do is compile your C programs with the `-g` flag.
```
cc -g [other flags] myprogram.c
```

**As of now, you will probably have to invoke `thorin` as root on MacOS. I will eventually figure out how to get code-signing to work with Rust binaries.**

## Why does this exist?
There is no real reason to use `thorin` over gdb, lldb or other similar more powerful debuggers. I wrote this because
1. I wanted to learn Rust.
2. It was a fun exercise in system programming.

I chose the name 'thorin' because this program reads DWARF files (the predominant format for debugging information).

## TODO
- fix the CLI
- passing arguments to and changing stdin/stdout of target process
- general bugfixes and QOL improvements
- MacOS code-signing stuff

## Author(s)
- Ajay Tatachar (<ajaymt2@illinois.edu>)
