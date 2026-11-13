//! The padding of records (T-300) over the unix carrier: the sizes are the
//! buckets, the reader gets no padding, and a changed `pad` fails the check.

mod link_record_harness;

use podnet::link_record::Options;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use link_record_harness::*;

#[tokio::test]
async fn padding_gives_sizes_from_the_buckets() {
    let opts = Options {
        pad: true,
        ..Options::default()
    };
    let (mut d, mut l, seen) = chain(opts, Options::default(), pass(), pass()).await;
    for size in [1usize, 100, 128, 200, 600, 1500, 3000] {
        d.write_all(&vec![7u8; size]).await.unwrap();
        let mut got = vec![0u8; size];
        l.read_exact(&mut got).await.unwrap();
        assert_eq!(got, vec![7u8; size], "padding is removed before the reader");
    }
    let lengths: Vec<u32> = seen
        .to_c
        .lock()
        .unwrap()
        .iter()
        .map(|r| header(r).length)
        .collect();
    assert_eq!(lengths, vec![128, 128, 128, 512, 1024, 2048, 4096]);
}

#[tokio::test]
async fn a_tampered_pad_fails_authentication() {
    let pad: Edit = Box::new(|i, mut r| {
        if i == 0 {
            // `pad` is bytes 20 and 21 of the header: say one byte less.
            let p = u16::from_be_bytes([r[20], r[21]]) - 1;
            r[20..22].copy_from_slice(&p.to_be_bytes());
        }
        vec![r]
    });
    let opts = Options {
        pad: true,
        ..Options::default()
    };
    let (mut d, mut l, _) = chain(opts, Options::default(), pad, pass()).await;
    d.write_all(b"hello").await.unwrap();
    let err = l.read(&mut [0u8; 16]).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
    assert!(err.to_string().contains("check"), "{err}");
}
