use torchflower_network::connection::controller::window::ReliableWindow;
use torchflower_network::connection::queue::{OrderedQueue, RecvQueue};
use torchflower_network::protocol::frame::FramePacket;

#[test]
fn ordered_queue_holds_later_packets_until_missing_index_arrives() {
    let mut queue = OrderedQueue::new();

    assert!(queue.insert(0, b"zero".to_vec()));
    assert_eq!(queue.flush(), vec![b"zero".to_vec()]);

    assert!(queue.insert(2, b"two".to_vec()));
    assert!(queue.flush().is_empty());

    assert!(queue.insert(1, b"one".to_vec()));
    assert_eq!(queue.flush(), vec![b"one".to_vec(), b"two".to_vec()]);
}

#[test]
fn ordered_queue_can_skip_abandoned_gap_and_flush_later_packets() {
    let mut queue = OrderedQueue::new();

    assert!(queue.insert(0, b"zero".to_vec()));
    assert_eq!(queue.flush(), vec![b"zero".to_vec()]);

    assert!(queue.insert(2, b"two".to_vec()));
    assert!(queue.insert(3, b"three".to_vec()));
    assert!(queue.flush().is_empty());

    queue.skip_through(1);
    assert_eq!(queue.flush(), vec![b"two".to_vec(), b"three".to_vec()]);
}

#[test]
fn reliable_window_accepts_bedrock_chunk_burst_beyond_legacy_size() {
    let mut window = ReliableWindow::with_size(16_384);
    window.seed_start(1_020);

    assert!(window.insert(3_068));
    assert!(window.insert(3_069));
    assert!(window.insert(3_070));

    assert_eq!(window.range(), (1_020, 17_404));
}

#[test]
fn reliable_window_reports_received_out_of_order_entries() {
    let mut window = ReliableWindow::with_size(8);
    window.seed_start(10);

    assert!(window.insert(11));
    assert!(window.contains(11));
    assert!(!window.contains(10));
}

#[test]
fn reliable_window_skip_through_advances_after_abandoned_gap() {
    let mut window = ReliableWindow::with_size(8);
    window.seed_start(10);

    assert!(window.insert(11));
    assert!(window.insert(12));
    assert_eq!(window.range(), (10, 18));

    window.skip_through(10);
    assert_eq!(window.range(), (13, 21));
}

#[test]
fn recv_queue_nacks_only_truly_missing_sequences() {
    let mut queue = RecvQueue::new();

    insert_sequence(&mut queue, 100);
    insert_sequence(&mut queue, 101);
    insert_sequence(&mut queue, 103);
    insert_sequence(&mut queue, 110);

    let mut nacks = queue.nack_queue();
    nacks.sort_unstable();

    assert!(nacks.contains(&102));
    assert!(!nacks.contains(&103));
    assert!(!nacks.contains(&110));
}

fn insert_sequence(queue: &mut RecvQueue, sequence: u32) {
    let mut packet = FramePacket::new();
    packet.sequence = sequence;
    queue.insert(packet).expect("insert frame packet");
}
