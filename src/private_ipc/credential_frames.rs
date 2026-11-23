//! Bounded private frames. No secret is represented by a serde value or an error message.
use crate::domain::provider_credentials::{
    CredentialResult, MAX_JIRA_SECRET_BYTES, Provider, ProviderCredentialError as Error,
    SecretToken,
};
use serde::{Serialize, de::DeserializeOwned};
use std::io::{Read, Write};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

pub(crate) const MAX_HEADER_BYTES: usize = 16 * 1024;
pub(crate) const MAX_SECRET_BYTES: usize = MAX_JIRA_SECRET_BYTES;
pub(crate) const FIRST_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
pub(crate) const SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub(crate) fn encode_header<T: Serialize>(value: &T) -> CredentialResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| Error::InvalidRequest)?;
    if bytes.len() >= MAX_HEADER_BYTES {
        return Err(Error::TooLarge);
    }
    bytes.push(b'\n');
    Ok(bytes)
}
pub(crate) fn read_header<R: Read, T: DeserializeOwned>(reader: &mut R) -> CredentialResult<T> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(256));
    loop {
        let mut byte = [0u8; 1];
        reader
            .read_exact(&mut byte)
            .map_err(|_| Error::Unavailable)?;
        if byte[0] == b'\n' {
            return serde_json::from_slice(&bytes).map_err(|_| Error::InvalidRequest);
        }
        if bytes.len() >= MAX_HEADER_BYTES - 1 {
            return Err(Error::TooLarge);
        }
        bytes.push(byte[0]);
    }
}
pub(crate) async fn read_header_async<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
) -> CredentialResult<T> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(256));
    loop {
        let byte = reader.read_u8().await.map_err(|_| Error::Unavailable)?;
        if byte == b'\n' {
            return serde_json::from_slice(&bytes).map_err(|_| Error::InvalidRequest);
        }
        if bytes.len() >= MAX_HEADER_BYTES - 1 {
            return Err(Error::TooLarge);
        }
        bytes.push(byte);
    }
}
pub(crate) fn write_header<W: Write, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> CredentialResult<()> {
    writer
        .write_all(&encode_header(value)?)
        .map_err(|_| Error::Unavailable)?;
    writer.flush().map_err(|_| Error::Unavailable)
}
pub(crate) async fn write_header_async<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> CredentialResult<()> {
    writer
        .write_all(&encode_header(value)?)
        .await
        .map_err(|_| Error::Unavailable)?;
    writer.flush().await.map_err(|_| Error::Unavailable)
}
fn secret_buffer(length: usize) -> CredentialResult<Zeroizing<Vec<u8>>> {
    if length == 0 {
        return Err(Error::InvalidRequest);
    }
    if length > MAX_SECRET_BYTES {
        return Err(Error::TooLarge);
    }
    Ok(Zeroizing::new(vec![0; length]))
}
pub(crate) fn read_secret<R: Read>(
    reader: &mut R,
    provider: Provider,
) -> CredentialResult<SecretToken> {
    let mut length = [0; 4];
    reader
        .read_exact(&mut length)
        .map_err(|_| Error::Unavailable)?;
    let mut bytes = secret_buffer(u32::from_be_bytes(length) as usize)?;
    reader
        .read_exact(&mut bytes)
        .map_err(|_| Error::Unavailable)?;
    SecretToken::from_bytes(provider, std::mem::take(&mut *bytes))
}
pub(crate) async fn read_secret_async<R: AsyncRead + Unpin>(
    reader: &mut R,
    provider: Provider,
) -> CredentialResult<SecretToken> {
    let length = reader.read_u32().await.map_err(|_| Error::Unavailable)?;
    let mut bytes = secret_buffer(length as usize)?;
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|_| Error::Unavailable)?;
    SecretToken::from_bytes(provider, std::mem::take(&mut *bytes))
}
pub(crate) fn write_secret<W: Write>(writer: &mut W, token: &SecretToken) -> CredentialResult<()> {
    writer
        .write_all(&(token.expose().len() as u32).to_be_bytes())
        .map_err(|_| Error::Unavailable)?;
    writer
        .write_all(token.expose())
        .map_err(|_| Error::Unavailable)?;
    writer.flush().map_err(|_| Error::Unavailable)
}
pub(crate) async fn write_secret_async<W: AsyncWrite + Unpin>(
    writer: &mut W,
    token: &SecretToken,
) -> CredentialResult<()> {
    writer
        .write_u32(token.expose().len() as u32)
        .await
        .map_err(|_| Error::Unavailable)?;
    writer
        .write_all(token.expose())
        .await
        .map_err(|_| Error::Unavailable)?;
    writer.flush().await.map_err(|_| Error::Unavailable)
}
pub(crate) fn require_eof<R: Read>(reader: &mut R) -> CredentialResult<()> {
    let mut byte = Zeroizing::new([0u8; 1]);
    match reader.read(&mut *byte) {
        Ok(0) => Ok(()),
        _ => Err(Error::InvalidRequest),
    }
}
pub(crate) async fn require_eof_async<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> CredentialResult<()> {
    let mut byte = Zeroizing::new([0u8; 1]);
    match reader.read(&mut *byte).await {
        Ok(0) => Ok(()),
        #[cfg(windows)]
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        _ => Err(Error::InvalidRequest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::io::Cursor;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Header {
        version: u8,
    }
    #[test]
    fn header_rejects_unknown_secret_field_without_echoing_input() {
        let mut input =
            Cursor::new(br#"{"version":1,"token":"synthetic-canary-credential"}"#.to_vec());
        input.get_mut().push(b'\n');
        let error = read_header::<_, Header>(&mut input).err().unwrap();
        assert_eq!(error, Error::InvalidRequest);
        assert!(!error.to_string().contains("synthetic-canary"));
    }
    #[test]
    fn secret_length_is_checked_before_allocation_and_truncation_is_rejected() {
        for length in [0, MAX_SECRET_BYTES as u32 + 1, u32::MAX] {
            assert!(read_secret(&mut Cursor::new(length.to_be_bytes()), Provider::Jira).is_err());
        }
        assert!(read_secret(&mut Cursor::new([0, 0, 0, 9, b'a']), Provider::Jira).is_err());
        let mut data = Vec::new();
        let token =
            SecretToken::from_bytes(Provider::Jira, b"synthetic-canary-credential".to_vec())
                .unwrap();
        write_secret(&mut data, &token).unwrap();
        let mut reader = Cursor::new(data);
        let actual = read_secret(&mut reader, Provider::Jira).unwrap();
        assert_eq!(actual.expose(), token.expose());
        require_eof(&mut reader).unwrap();
    }
    #[tokio::test]
    async fn async_headers_and_secret_frames_match_the_sync_encoding() {
        let mut header = &b"{\"version\":1}\n"[..];
        assert_eq!(
            read_header_async::<_, Header>(&mut header)
                .await
                .unwrap()
                .version,
            1
        );
        let token =
            SecretToken::from_bytes(Provider::Linear, b"synthetic-private-token".to_vec()).unwrap();
        let mut bytes = Vec::new();
        write_secret(&mut bytes, &token).unwrap();
        let mut reader = bytes.as_slice();
        assert_eq!(
            read_secret_async(&mut reader, Provider::Linear)
                .await
                .unwrap()
                .expose(),
            token.expose()
        );
        require_eof_async(&mut reader).await.unwrap();
        let oversized = vec![b'x'; MAX_HEADER_BYTES + 1];
        assert_eq!(
            read_header_async::<_, Header>(&mut oversized.as_slice())
                .await
                .err()
                .unwrap(),
            Error::TooLarge
        );
    }
}
