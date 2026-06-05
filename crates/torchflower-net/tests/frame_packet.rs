use binary_util::interfaces::Writer;
use torchflower_net::protocol::{
    frame::{FragmentMeta, Frame, FramePacket},
    reliability::Reliability,
};

#[test]
fn frame_packet_uses_prismarine_compatible_datagram_id() {
    let mut packet = FramePacket::new();
    packet.sequence = 7;
    packet.reliability = Reliability::ReliableOrd;

    let mut frame = Frame::new(Reliability::ReliableOrd, Some(&[0xfe, 0x01, 0x02]));
    frame.reliable_index = Some(3);
    frame.order_index = Some(1);
    frame.order_channel = Some(0);
    packet.frames.push(frame);

    let encoded = packet.write_to_bytes().expect("encode FramePacket");
    assert_eq!(encoded.as_slice()[0], 0x80);
}

#[test]
fn fragmented_frame_uses_prismarine_compatible_split_metadata() {
    let mut packet = FramePacket::new();
    packet.sequence = 7;
    packet.reliability = Reliability::ReliableOrd;

    let mut frame = Frame::new(Reliability::ReliableOrd, Some(&[0xfe, 0x01, 0x02]));
    frame.reliable_index = Some(3);
    frame.order_index = Some(1);
    frame.order_channel = Some(0);
    frame.fragment_meta = Some(FragmentMeta::new(21, 1, 5));
    packet.frames.push(frame);

    let encoded = packet.write_to_bytes().expect("encode split FramePacket");

    let bytes = encoded.as_slice();
    assert_eq!(bytes[0], 0x80);
    assert_eq!(&bytes[1..4], &[0x07, 0x00, 0x00]);
    assert_eq!(bytes[4], 0x70);
    assert_eq!(&bytes[5..7], &[0x00, 0x18]);
    assert_eq!(&bytes[7..10], &[0x03, 0x00, 0x00]);
    assert_eq!(&bytes[10..13], &[0x01, 0x00, 0x00]);
    assert_eq!(bytes[13], 0x00);
    assert_eq!(&bytes[14..18], &[0x00, 0x00, 0x00, 0x15]);
    assert_eq!(&bytes[18..20], &[0x00, 0x01]);
    assert_eq!(&bytes[20..24], &[0x00, 0x00, 0x00, 0x05]);
    assert_eq!(&bytes[24..], &[0xfe, 0x01, 0x02]);
}
