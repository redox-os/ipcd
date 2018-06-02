use std::collections::BTreeMap;
use syscall::{flag::*, error::*, Error, SchemeBlockMut, Result};

pub struct Handle {
    path: Option<String>,
    remote: Option<usize>,
    buffer: Vec<u8>
}
impl Handle {
    pub fn dup(&mut self) -> Self {
        Self {
            path: None,
            remote: self.remote.take(),
            buffer: Vec::new()
        }
    }
}

#[derive(Default)]
pub struct ChanScheme {
    handles: BTreeMap<usize, Handle>,
    listeners: BTreeMap<String, usize>,
    next_id: usize
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

        let mut handle = Handle {
            path: None,
            remote: None,
            buffer: Vec::new()
        };

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
                let mut remote = match self.handles.get(&id) {
                    Some(ref handle) if handle.path.is_some() => handle.remote,
                    _ => return Err(Error::new(EBADF))
                };
                if let Some(remote) = remote {
                    let new_id = self.next_id;
                    let mut clone = self.handles.get_mut(&id).map(Handle::dup).unwrap();

                    self.handles.insert(new_id, clone);
                    self.next_id += 1;

                    let mut remote = self.handles.get_mut(&remote).unwrap();
                    remote.remote = Some(new_id);
                    Ok(Some(new_id))
                } else {
                    Ok(None)
                }
            },
            _ => {
                return Err(Error::new(EBADF));
            }
        }
    }
    fn write(&mut self, id: usize, buf: &[u8]) -> Result<Option<usize>> {
        let remote = match self.handles.get(&id) {
            Some(handle) if handle.path.is_none() => handle.remote,
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
        match self.handles.get(&id) {
            Some(handle) if handle.path.is_none() => Ok(Some(id)),
            _ => Err(Error::new(EBADF))
        }
    }
    fn read(&mut self, id: usize, buf: &mut [u8]) -> Result<Option<usize>> {
        let handle = match self.handles.get_mut(&id) {
            Some(handle) => handle,
            None => return Err(Error::new(EBADF))
        };
        if handle.path.is_some() {
            // This is a listener, not a stream.
            Err(Error::new(EBADF))
        } else if handle.buffer.is_empty() {
            Ok(None)
        } else {
            let len = buf.len().min(handle.buffer.len());
            buf[..len].copy_from_slice(&handle.buffer[..len]);
            handle.buffer.drain(..len);
            Ok(Some(len))
        }
    }
    fn close(&mut self, id: usize) -> Result<Option<usize>> {
        let handle = match self.handles.remove(&id) {
            Some(handle) => handle,
            None => return Err(Error::new(EBADF))
        };

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
