//! Noise manages key agreement, directional keys and ordered-message nonces.
use crate::StoreError;
use snow::{Builder, HandshakeState, TransportState};

pub const SUBPROTOCOL: &str = "testflying.store.v2";
pub const CHUNK_BYTES: usize = 32 * 1024;
pub const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RECORD_BYTES: usize = CHUNK_BYTES + 17;
pub const HANDSHAKE_BYTES: usize = 4096;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Registration {
    pub connector_id: String,
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs_version: Option<u8>,
}

pub fn error() -> StoreError {
    StoreError::new(
        400,
        "encrypted_channel_invalid",
        "Encrypted channel authentication or framing failed",
    )
}

pub fn handshake(
    key: &str,
    connector_id: &str,
    initiator: bool,
) -> Result<HandshakeState, StoreError> {
    let key: [u8; 32] = hex::decode(key)
        .map_err(|_| error())?
        .try_into()
        .map_err(|_| error())?;
    let params = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s"
        .parse()
        .map_err(|_| error())?;
    let prologue = format!("{SUBPROTOCOL}:{connector_id}");
    let builder = Builder::new(params)
        .prologue(prologue.as_bytes())
        .map_err(|_| error())?
        .psk(0, &key)
        .map_err(|_| error())?;
    if initiator {
        builder.build_initiator()
    } else {
        builder.build_responder()
    }
    .map_err(|_| error())
}

pub struct Channel {
    state: TransportState,
    partial: Option<Vec<u8>>,
}

impl Channel {
    pub fn new(state: HandshakeState) -> Result<Self, StoreError> {
        Ok(Self {
            state: state.into_transport_mode().map_err(|_| error())?,
            partial: None,
        })
    }

    pub fn encrypt(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, StoreError> {
        if bytes.is_empty() || bytes.len() > MAX_JSON_BYTES {
            return Err(error());
        }
        let count = bytes.len().div_ceil(CHUNK_BYTES);
        bytes
            .chunks(CHUNK_BYTES)
            .enumerate()
            .map(|(index, part)| {
                let mut plain = Vec::with_capacity(part.len() + 1);
                plain.push(u8::from(index == 0) * 2 + u8::from(index + 1 == count));
                plain.extend_from_slice(part);
                let mut encrypted = vec![0; plain.len() + 16];
                let size = self
                    .state
                    .write_message(&plain, &mut encrypted)
                    .map_err(|_| error())?;
                encrypted.truncate(size);
                Ok(encrypted)
            })
            .collect()
    }

    pub fn decrypt(&mut self, bytes: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        if bytes.len() < 17 || bytes.len() > MAX_RECORD_BYTES {
            return Err(error());
        }
        let mut plain = vec![0; bytes.len()];
        let size = self
            .state
            .read_message(bytes, &mut plain)
            .map_err(|_| error())?;
        if size < 2 || plain[0] > 3 {
            return Err(error());
        }
        let flags = plain[0];
        if flags & 2 != 0 {
            if self.partial.is_some() {
                return Err(error());
            }
            self.partial = Some(Vec::new());
        }
        let partial = self.partial.as_mut().ok_or_else(error)?;
        if partial.len() + size - 1 > MAX_JSON_BYTES {
            return Err(error());
        }
        partial.extend_from_slice(&plain[1..size]);
        Ok(if flags & 1 != 0 {
            self.partial.take()
        } else {
            None
        })
    }
}
