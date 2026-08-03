use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::WispError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketType {
    Connect = 0x01,
    Data = 0x02,
    Continue = 0x03,
    Close = 0x04,
    Info = 0x05,
}

impl PacketType {
    pub fn from_u8(value: u8) -> Result<Self, WispError> {
        match value {
            0x01 => Ok(PacketType::Connect),
            0x02 => Ok(PacketType::Data),
            0x03 => Ok(PacketType::Continue),
            0x04 => Ok(PacketType::Close),
            0x05 => Ok(PacketType::Info),
            _ => Err(WispError::InvalidPacketType(value)),
        }
    }
}

pub type StreamId = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub packet_type: PacketType,
    pub stream_id: StreamId,
    pub payload: Bytes,
}

impl Packet {
    pub fn parse(data: Bytes) -> Result<Self, WispError> {
        if data.len() < 5 {
            return Err(WispError::PacketTooShort(data.len()));
        }

        let mut buf = data;
        let packet_type = PacketType::from_u8(buf.get_u8())?;
        let stream_id = buf.get_u32_le();
        let payload = buf.copy_to_bytes(buf.remaining());

        Ok(Packet { packet_type, stream_id, payload })
    }

    pub fn serialize(&self) -> Bytes {
        let len = 5 + self.payload.len();
        let mut buf = BytesMut::with_capacity(len);
        buf.put_u8(self.packet_type as u8);
        buf.put_u32_le(self.stream_id);
        buf.put_slice(&self.payload);
        buf.freeze()
    }

    /// Optimized serialization for DATA packets — avoids creating intermediate Packet struct.
    #[inline]
    pub fn serialize_data(stream_id: StreamId, payload: Bytes) -> Bytes {
        let len = 5 + payload.len();
        let mut buf = BytesMut::with_capacity(len);
        buf.put_u8(PacketType::Data as u8);
        buf.put_u32_le(stream_id);
        buf.put_slice(&payload);
        buf.freeze()
    }

    pub fn data(stream_id: StreamId, payload: Bytes) -> Self {
        Packet { packet_type: PacketType::Data, stream_id, payload }
    }

    pub fn continue_packet(stream_id: StreamId, buffer_remaining: u32) -> Self {
        let mut payload = BytesMut::with_capacity(4);
        payload.put_u32_le(buffer_remaining);
        Packet { packet_type: PacketType::Continue, stream_id, payload: payload.freeze() }
    }

    pub fn close(stream_id: StreamId, reason: u8) -> Self {
        let payload = Bytes::from(vec![reason]);
        Packet { packet_type: PacketType::Close, stream_id, payload }
    }

    pub fn info(stream_id: StreamId, major: u8, minor: u8, extensions: &[u8]) -> Self {
        let mut payload = BytesMut::with_capacity(2 + extensions.len());
        payload.put_u8(major);
        payload.put_u8(minor);
        payload.put_slice(extensions);
        Packet { packet_type: PacketType::Info, stream_id, payload: payload.freeze() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_data_packet() {
        let mut data = BytesMut::new();
        data.put_u8(0x02);
        data.put_u32_le(42);
        data.put_slice(b"hello");

        let packet = Packet::parse(data.freeze()).unwrap();
        assert_eq!(packet.packet_type, PacketType::Data);
        assert_eq!(packet.stream_id, 42);
        assert_eq!(packet.payload.as_ref(), b"hello");
    }

    #[test]
    fn test_parse_too_short() {
        let data = Bytes::from(vec![0x01, 0x00, 0x00]);
        let result = Packet::parse(data);
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_roundtrip() {
        let packet = Packet::data(7, Bytes::from_static(b"test"));
        let serialized = packet.serialize();
        let parsed = Packet::parse(serialized).unwrap();
        assert_eq!(packet, parsed);
    }

    #[test]
    fn test_continue_packet() {
        let packet = Packet::continue_packet(5, 1024);
        let serialized = packet.serialize();
        let parsed = Packet::parse(serialized).unwrap();
        assert_eq!(parsed.packet_type, PacketType::Continue);
        assert_eq!(parsed.payload.as_ref(), 1024u32.to_le_bytes());
    }

    #[test]
    fn test_close_packet() {
        let packet = Packet::close(3, 0x01);
        assert_eq!(packet.payload.as_ref(), &[0x01]);
    }

    #[test]
    fn test_info_packet() {
        let packet = Packet::info(0, 2, 1, &[0x10, 0x20]);
        assert_eq!(packet.payload.as_ref(), &[2, 1, 0x10, 0x20]);
    }
}
