use std::{
    cmp,
    collections::{HashMap, hash_map::Entry},
    rc::Rc,
};
use syscall::{error::*, Error, Map, Result, MapFlags, PAGE_SIZE, MAP_PRIVATE};
use redox_scheme::{SchemeMut, V2};

#[derive(Default)]
pub struct ShmHandle {
    buffer: Option<MmapGuard>,
    refs: usize
}
pub struct ShmScheme {
    maps: HashMap<Rc<str>, ShmHandle>,
    handles: HashMap<usize, Rc<str>>,
    next_id: usize,
    pub socket: redox_scheme::Socket,
}
impl ShmScheme {
    pub fn new() -> Result<Self> {
        Ok(Self {
            maps: HashMap::new(),
            handles: HashMap::new(),
            next_id: 0,
            socket: redox_scheme::Socket::<V2>::nonblock("shm")?,
        })
    }
}

impl SchemeMut for ShmScheme {
    fn open(&mut self, path: &str, _flags: usize, _uid: u32, _gid: u32) -> Result<usize> {
        let path = Rc::from(path);
        let entry = self.maps.entry(Rc::clone(&path)).or_insert(ShmHandle::default());
        entry.refs += 1;
        self.handles.insert(self.next_id, path);

        let id = self.next_id;
        self.next_id += 1;
        Ok(id)
    }
    fn fpath(&mut self, id: usize, buf: &mut [u8]) -> Result<usize> {
        // Write scheme name
        const PREFIX: &[u8] = b"shm:";
        let len = cmp::min(PREFIX.len(), buf.len());
        buf[..len].copy_from_slice(&PREFIX[..len]);
        if len < PREFIX.len() {
            return Ok(len);
        }

        // Write path
        let path = self.handles.get(&id).ok_or(Error::new(EBADF))?;
        let len = cmp::min(path.len(), buf.len() - PREFIX.len());
        buf[PREFIX.len()..][..len].copy_from_slice(&path.as_bytes()[..len]);

        Ok(PREFIX.len() + len)
    }
    fn close(&mut self, id: usize) -> Result<usize> {
        let path = self.handles.remove(&id).ok_or(Error::new(EBADF))?;
        let mut entry = match self.maps.entry(path) {
            Entry::Occupied(entry) => entry,
            Entry::Vacant(_) => panic!("handle pointing to nothing")
        };
        entry.get_mut().refs -= 1;
        if entry.get().refs == 0 {
            // There is no other reference to this entry, drop
            entry.remove_entry();
        }
        Ok(0)
    }
    fn mmap_prep(&mut self, id: usize, offset: u64, size: usize, _: MapFlags) -> Result<usize> {
        let path = self.handles.get(&id).ok_or(Error::new(EBADF))?;
        let total_size = offset as usize + size;
        match self.maps.get_mut(path).expect("handle pointing to nothing").buffer {
            Some(ref mut buf) => {
                if total_size > buf.len() {
                    return Err(Error::new(ERANGE));
                }
                Ok(buf.as_ptr() + offset as usize)
            },
            ref mut buf @ None => {
                *buf = Some(MmapGuard::alloc(size.div_ceil(PAGE_SIZE))?);
                Ok(buf.as_mut().unwrap().as_ptr() + offset as usize)
            }
        }
    }
}

pub struct MmapGuard {
    base: usize,
    size: usize,
}
impl MmapGuard {
    pub fn alloc(page_count: usize) -> Result<Self> {
        let size = page_count * PAGE_SIZE;
        let base = unsafe { syscall::fmap(!0, &Map { offset: 0, size, flags: MAP_PRIVATE, address: 0 }) }?;

        Ok(Self {
            base,
            size,
        })
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn as_ptr(&self) -> usize {
        self.base
    }
}
impl Drop for MmapGuard {
    fn drop(&mut self) {
        if self.size == 0 {
            return;
        }
        let _ = unsafe { syscall::funmap(self.base, self.size) };
    }
}
