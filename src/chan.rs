use crate::post_fevent;
use std::{
    cmp,
    collections::{HashMap, VecDeque},
    fs::File,
    io
};
use syscall::{flag::*, error::*, Error, SchemeBlockMut, Result};

#[derive(Debug, Default)]
pub struct Client {
    buffer: Vec<u8>,
    remote: Connection
}
#[derive(Debug, Default)]
pub struct Listener {
    path: Option<String>,
    awaiting: VecDeque<usize>
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
    extra: Extra
}
impl Handle {
    /// Duplicate this listener handle into one that is linked to the
    /// specified remote.
    /// Does NOT error if this is not a listener
    pub fn accept(&self, remote: usize) -> Self {
        Self {
            flags: self.flags,
            extra: Extra::Client(Client {
                remote: Connection::Open(remote),
                ..Client::default()
            }),
            ..Default::default()
        }
    }

    /// Mark this listener handle as having a connection which can be
    /// accepted, but only if it is ready to accept.
    /// Errors if this is not a listener
    pub fn connect(&mut self, other: usize) -> Result<()> {
        match self.extra {
            Extra::Listener(ref mut listener) => {
                listener.awaiting.push_back(other);
                Ok(())
            },
            _ => Err(Error::new(EBADF))
        }
    }

    /// Error if this is not a listener
    pub fn require_listener(&mut self) -> Result<&mut Listener> {
        match self.extra {
            Extra::Listener(ref mut listener) => Ok(listener),
            _ => Err(Error::new(EBADF))
        }
    }

    /// Error if this is not a client
    pub fn require_client(&mut self) -> Result<&mut Client> {
        match self.extra {
            Extra::Client(ref mut client) => Ok(client),
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
    //   ___  ____  _____ _   _
    //  / _ \|  _ \| ____| \ | |
    // | | | | |_) |  _| |  \| |
    // | |_| |  __/| |___| |\  |
    //  \___/|_|   |_____|_| \_|

    fn open(&mut self, path: &[u8], flags: usize, _uid: u32, _gid: u32) -> Result<Option<usize>> {
        let path = ::std::str::from_utf8(path).or(Err(Error::new(EPERM)))?;

        let new_id = self.next_id;
        let mut new = Handle::default();
        new.flags = flags;

        let create = flags & O_CREAT == O_CREAT;

        if create && !self.listeners.contains_key(path) {
            let mut listener = Listener::default();
            if !path.is_empty() {
                self.listeners.insert(String::from(path), new_id);
                listener.path = Some(String::from(path));
            }
            new.extra = Extra::Listener(listener);
        } else if create && flags & O_EXCL == O_EXCL {
            return Err(Error::new(EEXIST));
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
                loop {
                    let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;
                    let listener = handle.require_listener()?;

                    break if let Some(remote_id) = listener.awaiting.pop_front() {
                        let new_id = self.next_id;
                        let new = handle.accept(remote_id);

                        // Hook the remote side, assuming it's still
                        // connected, up to this one so the connection is
                        // mutal.
                        let remote = match self.handles.get_mut(&remote_id) {
                            Some(client) => client,
                            None => continue // Check next client
                        };
                        match remote.extra {
                            Extra::Client(ref mut client) => {
                                client.remote = Connection::Open(new_id);
                            },
                            Extra::Listener(_) => panic!("newly created handle can't possibly be a listener")
                        }
                        post_fevent(&mut self.socket, remote_id, EVENT_WRITE)?;

                        self.handles.insert(new_id, new);
                        self.next_id += 1;
                        Ok(Some(new_id))
                    } else if handle.flags & O_NONBLOCK == O_NONBLOCK {
                        Err(Error::new(EAGAIN))
                    } else {
                        Ok(None)
                    };
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

    //  ___ ___     ___      ____ _     ___  ____  _____
    // |_ _/ _ \   ( _ )    / ___| |   / _ \/ ___|| ____|
    //  | | | | |  / _ \/\ | |   | |  | | | \___ \|  _|
    //  | | |_| | | (_>  < | |___| |__| |_| |___) | |___
    // |___\___/   \___/\/  \____|_____\___/|____/|_____|

    fn write(&mut self, id: usize, buf: &[u8]) -> Result<Option<usize>> {
        let handle = self.handles.get_mut(&id).ok_or(Error::new(EBADF))?;
        let client = handle.require_client()?;

        if let Connection::Open(remote_id) = client.remote {
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
        } else if client.remote == Connection::Closed {
            Err(Error::new(EPIPE))
        } else if handle.flags & O_NONBLOCK == O_NONBLOCK {
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
        let client = handle.require_client()?;

        if !client.buffer.is_empty() {
            let len = cmp::min(buf.len(), client.buffer.len());
            buf[..len].copy_from_slice(&client.buffer[..len]);
            client.buffer.drain(..len);
            Ok(Some(len))
        } else if client.remote == Connection::Closed {
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
            Extra::Client(client) => if let Connection::Open(remote_id) = client.remote {
                let remote = self.handles.get_mut(&remote_id).unwrap();

                match remote.extra {
                    Extra::Client(ref mut client) => {
                        client.remote = Connection::Closed;
                        if client.buffer.is_empty() {
                            // Post readable on EOF only if it wasn't
                            // readable before
                            post_fevent(&mut self.socket, remote_id, EVENT_READ)?;
                        }
                    },
                    Extra::Listener(_) => panic!("a client can't be connected to a listener!")
                }
            },
            Extra::Listener(listener) => if let Some(path) = listener.path {
                self.listeners.remove(&path);
            }
        }
        Ok(Some(0))
    }


    //  ____   _    ____      _    __  __ _____ _____ _____ ____  ____
    // |  _ \ / \  |  _ \    / \  |  \/  | ____|_   _| ____|  _ \/ ___|
    // | |_) / _ \ | |_) |  / _ \ | |\/| |  _|   | | |  _| | |_) \___ \
    // |  __/ ___ \|  _ <  / ___ \| |  | | |___  | | | |___|  _ < ___) |
    // |_| /_/   \_\_| \_\/_/   \_\_|  |_|_____| |_| |_____|_| \_\____/

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
                if let Connection::Open(_) = client.remote {
                    events |= EVENT_WRITE;
                }
                if !client.buffer.is_empty() || client.remote == Connection::Closed {
                    events |= EVENT_READ;
                }
            },
            Extra::Listener(ref listener) => if !listener.awaiting.is_empty() {
                events |= EVENT_READ | EVENT_WRITE;
            }
        }
        Ok(Some(events))
    }
}
