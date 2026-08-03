use bytes::Bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Extension {
    Udp = 0x01,
    PasswordAuth = 0x02,
    KeyAuth = 0x03,
    Motd = 0x04,
    StreamOpenConfirmation = 0x05,
}

impl Extension {
    pub fn from_u8(id: u8) -> Option<Self> {
        match id {
            0x01 => Some(Self::Udp),
            0x02 => Some(Self::PasswordAuth),
            0x03 => Some(Self::KeyAuth),
            0x04 => Some(Self::Motd),
            0x05 => Some(Self::StreamOpenConfirmation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtensionNegotiation {
    pub server_supported: Vec<Extension>,
    pub client_supported: Vec<Extension>,
    pub agreed: Vec<Extension>,
}

impl ExtensionNegotiation {
    pub fn negotiate(server: &[Extension], client: &[Extension]) -> Self {
        let agreed: Vec<Extension> = server
            .iter()
            .copied()
            .filter(|ext| client.contains(ext))
            .collect();

        Self {
            server_supported: server.to_vec(),
            client_supported: client.to_vec(),
            agreed,
        }
    }

    pub fn has(&self, ext: Extension) -> bool {
        self.agreed.contains(&ext)
    }

    pub fn agreed_ids(&self) -> Vec<u8> {
        self.agreed.iter().map(|ext| *ext as u8).collect()
    }
}

#[derive(Debug, Clone)]
pub struct ExtensionData {
    pub id: u8,
    pub metadata: Bytes,
}

pub fn parse_extension_data(data: &[u8]) -> Vec<ExtensionData> {
    let mut extensions = Vec::new();
    let mut offset = 0;

    while offset + 5 <= data.len() {
        let id = data[offset];
        let length = u32::from_le_bytes([
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
        ]) as usize;
        offset += 5;

        if offset + length > data.len() {
            break;
        }

        let metadata = Bytes::copy_from_slice(&data[offset..offset + length]);
        offset += length;

        extensions.push(ExtensionData { id, metadata });
    }

    extensions
}
