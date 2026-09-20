use libnagi::storage::{BlockDevice, DirectoryEntry, FileHandle, StorageError, Vfs};
use nagi_net::{Device, Ipv4Address, NetError, SocketApi};

pub struct FileSystem<D: BlockDevice> {
    volume: Vfs<D>,
}

impl<D: BlockDevice> FileSystem<D> {
    pub const fn new(volume: Vfs<D>) -> Self {
        Self { volume }
    }

    pub fn mount_or_format(device: D) -> Result<(Self, bool), StorageError> {
        let (volume, formatted) = Vfs::mount_or_format(device)?;
        Ok((Self { volume }, formatted))
    }

    pub fn create(&mut self, name: &[u8]) -> Result<FileHandle, StorageError> {
        self.volume.create(name)
    }

    pub fn open(&mut self, name: &[u8]) -> Result<FileHandle, StorageError> {
        self.volume.open(name)
    }

    pub fn write(&mut self, handle: FileHandle, data: &[u8]) -> Result<(), StorageError> {
        self.volume.write(handle, data)
    }

    pub fn read(
        &mut self,
        handle: FileHandle,
        destination: &mut [u8],
    ) -> Result<usize, StorageError> {
        self.volume.read(handle, destination)
    }

    pub fn list_root(&mut self, entries: &mut [DirectoryEntry]) -> Result<usize, StorageError> {
        self.volume.list_root(entries)
    }

    pub fn volume_mut(&mut self) -> &mut Vfs<D> {
        &mut self.volume
    }
}

pub struct Network<D: Device> {
    stack: SocketApi<D>,
}

impl<D: Device> Network<D> {
    pub const fn new(device: D) -> Self {
        Self {
            stack: SocketApi::new(device),
        }
    }

    pub fn http_get(
        &mut self,
        target: Ipv4Address,
        target_port: u16,
        path: &[u8],
        expected_body: &[u8],
        response: &mut [u8],
    ) -> Result<usize, NetError> {
        self.stack
            .http_get(target, target_port, path, expected_body, response)
    }
}
