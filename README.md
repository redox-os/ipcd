# ipcd

A userspace daemon for interprocess communication.

## Usage

See the `examples` folder. [Hello World example](https://github.com/redox-os/ipcd/blob/master/examples/hello.rs)

Simply open `chan:<name>` with O_CREAT where `<name>` is any name you'd like to create a listener.  
This listener can accept clients by calling `dup("listen")`.

Open `chan:<name>` without O_CREAT to connect. Now you can read and write between both streams.
