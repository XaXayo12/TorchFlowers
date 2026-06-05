#![allow(unknown_lints)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use torchflower_proto::{
    compress, decompress, Compression, LoginPacket, MovePlayerPacket, PacketCodec, ProtocolVersion,
    TextPacket,
};

fn packet_codecs(c: &mut Criterion) {
    c.bench_function("login_encode", |b| {
        let packet = LoginPacket {
            protocol_version: 766,
            chain_json: "{\"chain\":[]}".to_string(),
            client_data_jwt: "client.jwt".to_string(),
        };
        b.iter(|| packet.encode(black_box(ProtocolVersion::V1_21_100)));
    });

    c.bench_function("text_encode_decode", |b| {
        let packet = TextPacket {
            source: "TorchFlower".to_string(),
            message: "hello".to_string(),
        };
        b.iter(|| {
            let mut encoded = packet.encode(ProtocolVersion::V1_21_100).unwrap();
            TextPacket::decode(&mut encoded, ProtocolVersion::V1_21_100).unwrap()
        });
    });

    c.bench_function("move_player_encode_decode", |b| {
        let packet = MovePlayerPacket {
            runtime_id: 42,
            position: [1.0, 64.0, 1.0],
            pitch: 0.0,
            yaw: 90.0,
            head_yaw: 90.0,
        };
        b.iter(|| {
            let mut encoded = packet.encode(ProtocolVersion::V1_21_100).unwrap();
            MovePlayerPacket::decode(&mut encoded, ProtocolVersion::V1_21_100).unwrap()
        });
    });
}

fn compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression");
    for size in [64usize, 512, 4096, 16384] {
        let payload = vec![b'A'; size];
        for algorithm in [
            ("zlib", Compression::Zlib { level: 1 }),
            ("zstd", Compression::Zstd { level: 1 }),
        ] {
            group.bench_with_input(
                BenchmarkId::new(format!("{}_round_trip", algorithm.0), size),
                &payload,
                |b, payload| {
                    b.iter(|| {
                        let compressed = compress(black_box(payload), algorithm.1, 1).unwrap();
                        decompress(&compressed, algorithm.1).unwrap()
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, packet_codecs, compression);
criterion_main!(benches);
