use std::io::{self, Read};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCryptCreateHash, BCryptDestroyHash,
    BCryptFinishHash, BCryptHash, BCryptHashData,
};

pub(crate) fn sha256_reader(mut input: impl Read) -> io::Result<[u8; 32]> {
    let hash = Sha256::new()?;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            return hash.finish();
        }
        hash.update(&buffer[..read])?;
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> io::Result<[u8; 32]> {
    let mut digest = [0; 32];
    check(unsafe {
        BCryptHash(BCRYPT_SHA256_ALG_HANDLE, null(), 0, bytes.as_ptr(), byte_count(bytes)?,
            digest.as_mut_ptr(), digest.len() as u32)
    })?;
    Ok(digest)
}

pub(crate) struct Sha256(BCRYPT_HASH_HANDLE);

impl Sha256 {
    pub(crate) fn new() -> io::Result<Self> {
        let mut handle = null_mut();
        check(unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut handle, null_mut(), 0, null(), 0, 0) })?;
        Ok(Self(handle))
    }

    pub(crate) fn update(&self, bytes: &[u8]) -> io::Result<()> {
        check(unsafe { BCryptHashData(self.0, bytes.as_ptr(), byte_count(bytes)?, 0) })
    }

    pub(crate) fn finish(self) -> io::Result<[u8; 32]> {
        let mut digest = [0; 32];
        check(unsafe { BCryptFinishHash(self.0, digest.as_mut_ptr(), digest.len() as u32, 0) })?;
        Ok(digest)
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        unsafe { BCryptDestroyHash(self.0) };
    }
}

fn byte_count(bytes: &[u8]) -> io::Result<u32> {
    u32::try_from(bytes.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "SHA-256 输入过长"))
}

fn check(status: i32) -> io::Result<()> {
    if status < 0 {
        return Err(io::Error::other(format!("Windows SHA-256 计算失败 ({status:#010x})")));
    }
    Ok(())
}
