use post_fevent;
use std::{
    collections::BTreeMap,
    fs::File,
    io
};
use syscall::{flag::*, error::*, Error, SchemeBlockMut, Result};

#[derive(Default)]
pub struct Handle {
    flags: usize,
    fevent: usize,
    notified_read: bool,
    notified_write: bool,

    path: Option<String>,
    remote: Option<usize>,
    buffer: Vec<u8>
}
impl Handle {
    pub fn accept(&mut self) -> Self {
        Self {
            flags: self.flags,
            remote: self.remote.take(),
            ..Default::default()
        }
    }
    pub fn is_listener(&self) -> bool {
        self.path.is_some()
    }
}

#[derive(Default)]
pub struct ChanScheme {
    handles: BTreeMap<usize, Handle>,
    listeners: BTreeMap<String, usize>,
    next_id: usize
}
impl ChanScheme {
    pub fn post_fevents(&mut self, file: &mut File) -> io::Result<()> {
        for (id, handle) in &mut self.handles {
            if !handle.notified_write {
                handle.notified_write = true;
                post_fevent(file, *id, EVENT_WRITE)?;
            }
            if !handle.buffer.is_empty() || (!handle.is_listener() && handle.remote.is_none()) {
                if !handle.notified_read {
                    handle.notified_read = true;
                    post_fevent(file, *id, EVENT_READ)?;
                }
            } else {
                handle.notified_read = false;
            }
        }
        Ok(())
    }
}
impl SchemeBlockMut for ChanScheme {
    fn open(&mut self, path: &[u8], flags: usize, _uid: u32, _gid: u32) -> Result<Option<usize>> {
        let path = ::std::str::from_utf8(path).unwrap_or("");
        if path.is_empty() {
            return Err(Error::new(EPERM));
        }
        if flags & O_CREAT == O_CREAT && self.listeners.contains_key(path) {
            return Err(Error::new(EADDRINUSE));
        } else if flags & O_CREAT != O_CREAT && !self.listeners.contains_key(path) {
            return Err(Error::new(ENOENT));
        }

        let mut handle = Handle::default();
        handle.flags = flags;

        let id = self.next_id;
        if flags & O_CREAT == O_CREAT {
            self.listeners.insert(String::from(path), id);
            handle.path = Some(String::from(path));
        } else {
            let listener = self.listeners[path];
            let handle = self.handles.get_mut(&listener).expect("orphan listener left over");
            handle.remote = Some(id);
        }
        self.handles.insert(id, handle);
        self.next_id += 1;
        Ok(Some(id))
    }
    fn dup(&mut self, id: usize, buf: &[u8]) -> Result<Option<usize>> {
        match buf {
            b"listen" => {
                let (flags, remote) = match self.handles.get(&id) {
                    Some(ref handle) if handle.is_listener() => (handle.flags, handle.remote),
                    _ => return Err(Error::new(EBADF))
                };
                if let Some(remote) = remote {
                    let new_id = self.next_id;
                    let mut clone = self.handles.get_mut(&id).map(Handle::accept).unwrap();

                    self.handles.insert(new_id, clone);
                    self.next_id += 1;

                    let mut remote = self.handles.get_mut(&remote).unwrap();
                    remote.remote = Some(new_id);
                    Ok(Some(new_id))
                } else if flags & O_NONBLOCK == O_NONBLOCK {
                    Err(Error::new(EAGAIN))
                } else {
                    Ok(None)
                }
            },
            _ => {
                return Err(Error::new(EBADF));
            }
        }
    }
    fn fcntl(&mut self, id: usize, cmd: usize, arg: usize) -> Result<Option<usize>> {
        let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;
        match cmd {
            F_GETFL => Ok(Some(handle.flags)),
            F_SETFL => {
                handle.flags = arg;
                Ok(Some(0))
            },
            _ => Err(Error::new(EINVAL))
        }
    }
    fn fevent(&mut self, id: usize, flags: usize) -> Result<Option<usize>> {
        let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;
        handle.fevent = flags;
        handle.notified_read = false;
        handle.notified_write = false;
        Ok(Some(id))
    }
    fn write(&mut self, id: usize, buf: &[u8]) -> Result<Option<usize>> {
        let remote = match self.handles.get(&id) {
            Some(handle) if !handle.is_listener() => handle.remote,
            _ => return Err(Error::new(EBADF))
        };
        if let Some(remote) = remote {
            let mut remote = self.handles.get_mut(&remote).unwrap();
            remote.buffer.extend(buf);
            Ok(Some(buf.len()))
        } else {
            Err(Error::new(ENOTCONN))
        }
    }
    fn fsync(&mut self, id: usize) -> Result<Option<usize>> {
        self.handles.get(&id)
            .ok_or(Error::new(EBADF))
            .and(Ok(Some(id)))
    }
    fn read(&mut self, id: usize, buf: &mut [u8]) -> Result<Option<usize>> {
        let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;

        if handle.is_listener() {
            Err(Error::new(EBADF))
        } else if !handle.buffer.is_empty() {
            let len = buf.len().min(handle.buffer.len());
            buf[..len].copy_from_slice(&handle.buffer[..len]);
            handle.buffer.drain(..len);
            Ok(Some(len))
        } else if handle.remote.is_none() {
            // Remote dropped, send EOF
            Ok(Some(0))
        } else if handle.flags & O_NONBLOCK == O_NONBLOCK {
            Err(Error::new(EAGAIN))
        } else {
            Ok(None)
        }
    }
    fn close(&mut self, id: usize) -> Result<Option<usize>> {
        let handle = self.handles.remove(&id).ok_or(Error::new(EBADF))?;

        if let Some(remote) = handle.remote {
            let mut remote = self.handles.get_mut(&remote).unwrap();
            remote.remote = None;
        }
        if let Some(path) = handle.path {
            self.listeners.remove(&path);
        }
        Ok(Some(0))
    }
}
