use post_fevent;
use std::{
    collections::BTreeMap,
    fs::File,
    io,
    mem
};
use syscall::{flag::*, error::*, Error, SchemeBlockMut, Result};

#[derive(Debug, Default)]
pub struct Client {
    buffer: Vec<u8>
}
#[derive(Debug, Default)]
pub struct Listener {
    path: Option<String>
}
#[derive(Debug)]
pub enum Extra {
    Client(Client),
    Listener(Listener)
}
impl Default for Extra {
    fn default() -> Self {
        Extra::Client(Client::default())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Connection {
    Waiting,
    Open(usize),
    Closed
}
impl Default for Connection {
    fn default() -> Self {
        Connection::Waiting
    }
}

#[derive(Debug, Default)]
pub struct Handle {
    flags: usize,
    notified_read: bool,
    notified_write: bool,

    remote: Connection,
    extra: Extra
}
impl Handle {
    pub fn accept(&mut self) -> Self {
        Self {
            flags: self.flags,
            remote: mem::replace(&mut self.remote, Connection::Waiting),
            ..Default::default()
        }
    }
    pub fn require_listener(&self) -> Result<()> {
        match self.extra {
            Extra::Listener(_) => Ok(()),
            _ => Err(Error::new(EBADF))
        }
    }
    pub fn require_client(&self) -> Result<()> {
        match self.extra {
            Extra::Client(_) => Ok(()),
            _ => Err(Error::new(EBADF))
        }
    }
}

#[derive(Default)]
pub struct IpcScheme {
    handles: BTreeMap<usize, Handle>,
    listeners: BTreeMap<String, usize>,
    next_id: usize
}
impl IpcScheme {
    pub fn post_fevents(&mut self, file: &mut File) -> io::Result<()> {
        for (id, handle) in &mut self.handles {
            match handle.extra {
                Extra::Listener(_) => {
                    if let Connection::Open(_) = handle.remote {
                        // Send writable because that's what smolnetd does for TcpListener
                        if !handle.notified_write {
                            handle.notified_write = true;
                            post_fevent(file, *id, EVENT_WRITE)?;
                        }
                    } else {
                        handle.notified_write = false;
                    }
                },
                Extra::Client(ref mut client) => {
                    if let Connection::Open(_) = handle.remote {
                        if !handle.notified_write {
                            handle.notified_write = true;
                            post_fevent(file, *id, EVENT_WRITE)?;
                        }
                    }
                    if !client.buffer.is_empty() || handle.remote == Connection::Closed {
                        if !handle.notified_read {
                            handle.notified_read = true;
                            post_fevent(file, *id, EVENT_READ)?;
                        }
                    } else {
                        handle.notified_read = false;
                    }
                }
            }
        }
        Ok(())
    }
}
impl SchemeBlockMut for IpcScheme {
    fn open(&mut self, path: &[u8], flags: usize, _uid: u32, _gid: u32) -> Result<Option<usize>> {
        let path = ::std::str::from_utf8(path).or(Err(Error::new(EPERM)))?;

        let mut new = Handle::default();
        new.flags = flags;

        let id = self.next_id;
        if flags & O_CREAT == O_CREAT {
            if self.listeners.contains_key(path) {
                return Err(Error::new(EADDRINUSE));
            }
            let mut listener = Listener::default();
            if !path.is_empty() {
                self.listeners.insert(String::from(path), id);
                listener.path = Some(String::from(path));
            }
            new.extra = Extra::Listener(listener);
        } else {
            let listener = self.listeners.get(path).ok_or(Error::new(ENOENT))?;
            let handle = self.handles.get_mut(&listener).expect("orphan listener left over");
            if handle.remote != Connection::Waiting {
                return Err(Error::new(ECONNREFUSED));
            }
            handle.remote = Connection::Open(id);
        }
        self.handles.insert(id, new);
        self.next_id += 1;
        Ok(Some(id))
    }
    fn dup(&mut self, id: usize, buf: &[u8]) -> Result<Option<usize>> {
        match buf {
            b"listen" => {
                let (flags, remote) = {
                    let handle = self.handles.get(&id).ok_or(Error::new(EBADF))?;
                    handle.require_listener()?;
                    (handle.flags, handle.remote)
                };
                if let Connection::Open(remote) = remote {
                    let new_id = self.next_id;

                    let mut clone = self.handles.get_mut(&id).map(Handle::accept).unwrap();

                    {
                        // This might fail if the remote side closed early
                        let mut remote = self.handles.get_mut(&remote).ok_or(Error::new(ECONNRESET))?;
                        remote.remote = Connection::Open(new_id);
                    }

                    self.handles.insert(new_id, clone);
                    self.next_id += 1;

                    Ok(Some(new_id))
                } else if flags & O_NONBLOCK == O_NONBLOCK {
                    Err(Error::new(EAGAIN))
                } else {
                    Ok(None)
                }
            },
            b"connect" => {
                let mut new = Handle::default();
                let new_id = self.next_id;

                {
                    let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;
                    handle.require_listener()?;

                    if handle.remote != Connection::Waiting {
                        return Err(Error::new(ECONNREFUSED));
                    }
                    handle.remote = Connection::Open(new_id);
                }

                self.handles.insert(new_id, new);
                self.next_id += 1;
                Ok(Some(new_id))
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
    fn fevent(&mut self, id: usize, _flags: usize) -> Result<Option<usize>> {
        let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;
        handle.notified_read = false;
        handle.notified_write = false;
        Ok(Some(id))
    }
    fn write(&mut self, id: usize, buf: &[u8]) -> Result<Option<usize>> {
        let (flags, remote) = {
            let handle = self.handles.get(&id).ok_or(Error::new(EBADF))?;
            handle.require_client()?;
            (handle.flags, handle.remote)
        };
        if let Connection::Open(remote) = remote {
            let mut remote = self.handles.get_mut(&remote).unwrap();
            match remote.extra {
                Extra::Client(ref mut client) => {
                    client.buffer.extend(buf);
                    Ok(Some(buf.len()))
                },
                Extra::Listener(_) => panic!("somehow, a client was connected to a listener directly")
            }
        } else if remote == Connection::Waiting && flags & O_NONBLOCK == O_NONBLOCK {
            Err(Error::new(EAGAIN))
        } else if remote == Connection::Waiting {
            Ok(None)
        } else {
            Err(Error::new(EPIPE))
        }
    }
    fn fsync(&mut self, id: usize) -> Result<Option<usize>> {
        self.handles.get(&id)
            .ok_or(Error::new(EBADF))
            .and(Ok(Some(id)))
    }
    fn read(&mut self, id: usize, buf: &mut [u8]) -> Result<Option<usize>> {
        let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;

        let client = match handle.extra {
            Extra::Client(ref mut client) => client,
            Extra::Listener(_) => return Err(Error::new(EBADF))
        };

        if !client.buffer.is_empty() {
            let len = buf.len().min(client.buffer.len());
            buf[..len].copy_from_slice(&client.buffer[..len]);
            client.buffer.drain(..len);
            Ok(Some(len))
        } else if handle.remote == Connection::Closed {
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

        match handle.extra {
            Extra::Client(_) => {
                if let Connection::Open(remote) = handle.remote {
                    let mut remote = self.handles.get_mut(&remote).unwrap();
                    remote.remote = Connection::Closed;
                }
            },
            Extra::Listener(listener) => {
                // Clients never register server's remote

                if let Some(path) = listener.path {
                    self.listeners.remove(&path);
                }
            }
        }
        Ok(Some(0))
    }
}
