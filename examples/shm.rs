use std::{
    fs::File,
    io,
    os::unix::io::AsRawFd,
    slice
};

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}
fn main() -> Result<(), io::Error> {
    let file1 = File::open("shm:example")?;
    let file2 = File::open("shm:example")?;

    let one = unsafe {
        slice::from_raw_parts_mut(
            syscall::fmap(file1.as_raw_fd() as usize, &syscall::Map {
                offset: 0,
                size: 128,
                flags: syscall::PROT_READ | syscall::PROT_WRITE | syscall::MAP_SHARED,
                address: 0,
            }).map_err(from_syscall_error)? as *mut u8,
            128
        )
    };
    // FIXME: While the length can be unaligned, the offset cannot. This test is incorrectly
    // written.
    let two = unsafe {
        slice::from_raw_parts_mut(
            syscall::fmap(file2.as_raw_fd() as usize, &syscall::Map {
                offset: 64,
                size: 64,
                flags: syscall::PROT_READ | syscall::PROT_WRITE | syscall::MAP_SHARED,
                address: 0,
            }).map_err(from_syscall_error)? as *mut u8,
            64
        )
    };

    println!("Testing writing between");
    for i in 0..128 {
        one[i as usize] = i;
    }
    for i in 0..64 {
        assert_eq!(two[i as usize], 64 + i);
    }

    println!("Testing fpath");
    let mut buf = [0; 128];
    let len = syscall::fpath(file1.as_raw_fd() as usize, &mut buf).map_err(from_syscall_error)?;
    assert_eq!(&buf[..len], b"shm:example");
    Ok(())
}
