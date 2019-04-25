use crate::post_fevent;
use std::{
    cmp,
    collections::HashMap,
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
    id: usize,
    flags: usize,
    remote: Connection,
    extra: Extra
}
impl Handle {
    /// Duplicate this listener handle (no checks done) into one that
    /// can be connected to.
    pub fn accept(&mut self) -> Self {
        Self {
            flags: self.flags,
            remote: mem::replace(&mut self.remote, Connection::Waiting),
            ..Default::default()
        }
    }

    /// Mark this listener handle (no checks done) as having a
    /// connection which can be accepted, but only if it is ready to
    /// accept.
    pub fn connect(&mut self, other: usize) -> Result<()> {
        if self.remote != Connection::Waiting {
            return Err(Error::new(ECONNREFUSED));
        }
        self.remote = Connection::Open(other);
        Ok(())
    }

    /// Error if this is not a listener
    pub fn require_listener(&self) -> Result<()> {
        match self.extra {
            Extra::Listener(_) => Ok(()),
            _ => Err(Error::new(EBADF))
        }
    }

    /// Error if this is not a client
    pub fn require_client(&self) -> Result<()> {
        match self.extra {
            Extra::Client(_) => Ok(()),
            _ => Err(Error::new(EBADF))
        }
    }
}

pub struct ChanScheme {
    handles: HashMap<usize, Handle>,
    listeners: HashMap<String, usize>,
    next_id: usize,
    pub socket: File
}
impl ChanScheme {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            handles: HashMap::new(),
            listeners: HashMap::new(),
            next_id: 0,
            socket: File::create(":chan")?
        })
    }
}

impl SchemeBlockMut for ChanScheme {
    fn open(&mut self, path: &[u8], flags: usize, _uid: u32, _gid: u32) -> Result<Option<usize>> {
        let path = ::std::str::from_utf8(path).or(Err(Error::new(EPERM)))?;

        let new_id = self.next_id;
        let mut new = Handle::default();
        new.flags = flags;

        if flags & O_CREAT == O_CREAT {
            if self.listeners.contains_key(path) {
                return Err(Error::new(EADDRINUSE));
            }
            let mut listener = Listener::default();
            if !path.is_empty() {
                self.listeners.insert(String::from(path), new_id);
                listener.path = Some(String::from(path));
            }
            new.extra = Extra::Listener(listener);
        } else {
            let listener_id = *self.listeners.get(path).ok_or(Error::new(ENOENT))?;
            let listener = self.handles.get_mut(&listener_id).expect("orphan listener left over");
            listener.connect(new_id)?;

            // smoltcp sends writeable whenever a listener gets a
            // client, we'll do the same too (but also readable, why
            // not)
            post_fevent(&mut self.socket, listener_id, EVENT_READ | EVENT_WRITE)?;
        }

        self.handles.insert(new_id, new);
        self.next_id += 1;
        Ok(Some(new_id))
    }
    fn dup(&mut self, id: usize, buf: &[u8]) -> Result<Option<usize>> {
        match buf {
            b"listen" => {
                let (flags, remote) = {
                    let handle = self.handles.get(&id).ok_or(Error::new(EBADF))?;
                    handle.require_listener()?;
                    (handle.flags, handle.remote)
                };
                if let Connection::Open(remote_id) = remote {
                    let new_id = self.next_id;
                    let new = self.handles.get_mut(&id).map(Handle::accept).unwrap();

                    // Hook the remote side, assuming it's still
                    // connected, up to this one so the connection is
                    // mutal.
                    let mut remote = self.handles.get_mut(&remote_id).ok_or(Error::new(ECONNRESET))?;
                    remote.remote = Connection::Open(new_id);
                    post_fevent(&mut self.socket, remote_id, EVENT_WRITE)?;

                    self.handles.insert(new_id, new);
                    self.next_id += 1;
                    Ok(Some(new_id))
                } else if flags & O_NONBLOCK == O_NONBLOCK {
                    Err(Error::new(EAGAIN))
                } else {
                    Ok(None)
                }
            },
            b"connect" => {
                let new_id = self.next_id;
                let new = Handle::default();

                let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;
                handle.require_listener()?;
                handle.connect(new_id)?;

                // smoltcp sends writeable whenever a listener gets a
                // client, we'll do the same too (but also readable,
                // why not)
                post_fevent(&mut self.socket, id, EVENT_READ | EVENT_WRITE)?;

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
        let mut events = 0;
        match handle.extra {
            Extra::Client(ref client) => {
                if let Connection::Open(_) = handle.remote {
                    events |= EVENT_WRITE;
                }
                if !client.buffer.is_empty() || handle.remote == Connection::Closed {
                    events |= EVENT_READ;
                }
            },
            Extra::Listener(_) => if let Connection::Open(_) = handle.remote {
                events |= EVENT_READ;
            }
        }
        Ok(Some(events))
    }
    fn write(&mut self, id: usize, buf: &[u8]) -> Result<Option<usize>> {
        let (flags, remote) = {
            let handle = self.handles.get(&id).ok_or(Error::new(EBADF))?;
            handle.require_client()?;
            (handle.flags, handle.remote)
        };
        if let Connection::Open(remote_id) = remote {
            let remote = self.handles.get_mut(&remote_id).unwrap();
            match remote.extra {
                Extra::Client(ref mut client) => {
                    client.buffer.extend(buf);
                    if client.buffer.len() == buf.len() {
                        // Send readable only if it wasn't readable
                        // before
                        post_fevent(&mut self.socket, remote_id, EVENT_READ)?;
                    }
                    Ok(Some(buf.len()))
                },
                Extra::Listener(_) => panic!("somehow, a client was connected to a listener directly")
            }
        } else if remote == Connection::Closed {
            Err(Error::new(EPIPE))
        } else if flags & O_NONBLOCK == O_NONBLOCK {
            Err(Error::new(EAGAIN))
        } else {
            Ok(None)
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
            let len = cmp::min(buf.len(), client.buffer.len());
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
                if let Connection::Open(remote_id) = handle.remote {
                    let mut remote = self.handles.get_mut(&remote_id).unwrap();
                    remote.remote = Connection::Closed;
                    match remote.extra {
                        Extra::Client(ref client) => if client.buffer.is_empty() {
                            // Post readable on EOF only if it wasn't
                            // readable before
                            post_fevent(&mut self.socket, remote_id, EVENT_READ)?;
                        },
                        Extra::Listener(_) => panic!("a client can't be connected to a listener!")
                    }
                }
            },
            Extra::Listener(listener) => {
                // Clients never reference listeners in any way, it's
                // safe to drop

                if let Some(path) = listener.path {
                    self.listeners.remove(&path);
                }
            }
        }
        Ok(Some(0))
    }
}
