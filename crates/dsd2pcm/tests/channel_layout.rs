//! Channel layouts: DSF channel types and DSDIFF channel IDs become the WAV channel mask of
//! a conversion; WAV channel masks read back; the DSF writer takes its type from a mask.

use dsd2pcm::dff::{DffReader, DffWriter};
use dsd2pcm::dsf::{channel_mask_for_type, channel_type_for_mask, DsfReader, DsfWriter};
use dsd2pcm::io::{DsdRead, DsdWrite};
use dsd2pcm::wav::{WavFormat, WavReader, WavWriter};
use dsd2pcm::{convert_file, PcmOptions, PcmRate};
use std::io::Cursor;
use std::path::{Path, PathBuf};

const BYTES: usize = 8192;

fn chans(ch: u16) -> Vec<Vec<u8>> {
    (0..ch)
        .map(|c| {
            (0..BYTES)
                .map(|i| (i * 5 + usize::from(c)) as u8 ^ 0x69)
                .collect()
        })
        .collect()
}

fn dsf(ch: u16, channel_type: Option<u32>) -> Vec<u8> {
    let mut w = DsfWriter::new(Cursor::new(Vec::new()), ch, 2_822_400, channel_type).unwrap();
    w.write_bytes(&chans(ch)).unwrap();
    w.finish(BYTES as u64 * 8).unwrap();
    w.into_inner().into_inner()
}

/// A DSDIFF file whose `CHNL` chunk lists `ids`.
fn dff(ids: &[&[u8; 4]]) -> Vec<u8> {
    let ch = ids.len();
    let mut chnl = (ch as u16).to_be_bytes().to_vec();
    for id in ids {
        chnl.extend_from_slice(*id);
    }
    let mut prop = b"SND ".to_vec();
    let cmpr = b"DSD \x0enot compressed\x00".to_vec();
    for (id, body) in [
        (b"FS  ", 2_822_400u32.to_be_bytes().to_vec()),
        (b"CHNL", chnl),
        (b"CMPR", cmpr),
    ] {
        prop.extend_from_slice(id);
        prop.extend_from_slice(&(body.len() as u64).to_be_bytes());
        prop.extend_from_slice(&body);
    }
    let c = chans(ch as u16);
    let mut data = Vec::with_capacity(BYTES * ch);
    for i in 0..BYTES {
        for x in &c {
            data.push(x[i]);
        }
    }
    let mut body = b"DSD ".to_vec();
    body.extend_from_slice(b"FVER");
    body.extend_from_slice(&4u64.to_be_bytes());
    body.extend_from_slice(&0x0105_0000u32.to_be_bytes());
    body.extend_from_slice(b"PROP");
    body.extend_from_slice(&(prop.len() as u64).to_be_bytes());
    body.extend_from_slice(&prop);
    body.extend_from_slice(b"DSD ");
    body.extend_from_slice(&(data.len() as u64).to_be_bytes());
    body.extend_from_slice(&data);
    let mut f = b"FRM8".to_vec();
    f.extend_from_slice(&(body.len() as u64).to_be_bytes());
    f.extend_from_slice(&body);
    f
}

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("channel-layout-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Converts `input` (DSF or DFF bytes) and returns the WAV's channel mask.
fn wav_mask(d: &Path, input: &[u8]) -> Option<u32> {
    let (inp, out) = (d.join("in"), d.join("out.wav"));
    std::fs::write(&inp, input).unwrap();
    convert_file(&inp, &out, &PcmOptions::new(PcmRate::Hz88200)).unwrap();
    let r = WavReader::new(Cursor::new(std::fs::read(&out).unwrap())).unwrap();
    assert_eq!(r.format(), WavFormat::I24);
    r.channel_mask()
}

#[test]
fn dsf_channel_types_set_the_wav_mask() {
    let d = dir("dsf");
    // (type, channels, mask): types 5, 6 and 7 of DSF 1.01 and the others.
    for (t, ch, mask) in [
        (5, 4, 0x0F), // FL FR C LFE: not the 4-channel default (quad)
        (6, 5, 0x37), // FL FR C BL BR
        (7, 6, 0x3F), // FL FR C LFE BL BR
        (4, 4, 0x33),
        (3, 3, 0x07),
        (2, 2, 0x03),
        (1, 1, 0x04),
    ] {
        let f = dsf(ch, Some(t));
        let r = DsfReader::new(Cursor::new(&f)).unwrap();
        assert_eq!(r.channel_type(), Some(t));
        assert_eq!(r.channel_mask(), Some(mask), "type {t}");
        assert_eq!(wav_mask(&d, &f), Some(mask), "type {t}");
    }
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn dsf_type_that_does_not_match_the_count_falls_back_to_the_count() {
    let d = dir("dsf-mismatch");
    // Type 7 (six channels) on a stereo file; type 5 (four) on a six-channel one.
    let mut f = dsf(2, None);
    f[28 + 12 + 8..28 + 12 + 12].copy_from_slice(&7u32.to_le_bytes());
    let r = DsfReader::new(Cursor::new(&f)).unwrap();
    assert_eq!((r.channel_type(), r.channel_mask()), (Some(7), None));
    assert_eq!(wav_mask(&d, &f), Some(0x03));
    let mut f = dsf(6, None);
    f[28 + 12 + 8..28 + 12 + 12].copy_from_slice(&5u32.to_le_bytes());
    assert_eq!(wav_mask(&d, &f), Some(0x3F));
    // An unknown type.
    let mut f = dsf(4, None);
    f[28 + 12 + 8..28 + 12 + 12].copy_from_slice(&9u32.to_le_bytes());
    assert_eq!(wav_mask(&d, &f), Some(0x33));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn dff_channel_ids_set_the_wav_mask() {
    let d = dir("dff");
    for (ids, mask, wav) in [
        (&[b"MLFT", b"MRGT", b"C   ", b"LFE "][..], Some(0x0F), 0x0F),
        (
            &[b"MLFT", b"MRGT", b"C   ", b"LS  ", b"RS  "][..],
            Some(0x37),
            0x37,
        ),
        (
            &[b"MLFT", b"MRGT", b"C   ", b"LFE ", b"LS  ", b"RS  "][..],
            Some(0x3F),
            0x3F,
        ),
        (&[b"MLFT", b"MRGT", b"LS  ", b"RS  "][..], Some(0x33), 0x33),
        (&[b"SLFT", b"SRGT"][..], Some(0x03), 0x03),
        (&[b"C   "][..], Some(0x04), 0x04),
        // Front left and right with the centre only: not one of the count's defaults.
        (&[b"SLFT", b"SRGT", b"LFE "][..], Some(0x0B), 0x0B),
        // Numbered IDs, an order WAV cannot express, a repeated speaker: the count's mask.
        (&[b"C000", b"C001", b"C002"][..], None, 0x07),
        (&[b"C   ", b"MLFT", b"MRGT"][..], None, 0x07),
        (&[b"MLFT", b"SLFT"][..], None, 0x03),
    ] {
        let f = dff(ids);
        let r = DffReader::new(Cursor::new(&f)).unwrap();
        assert_eq!(r.channel_mask(), mask, "{ids:?}");
        assert_eq!(wav_mask(&d, &f), Some(wav), "{ids:?}");
    }
    // The writer's own IDs read back.
    for (ch, mask) in [
        (1, Some(0x04)),
        (2, Some(0x03)),
        (5, Some(0x37)),
        (6, Some(0x3F)),
        (3, None),
    ] {
        let mut w = DffWriter::new(Cursor::new(Vec::new()), ch, 2_822_400).unwrap();
        w.write_bytes(&chans(ch)).unwrap();
        w.finish(BYTES as u64 * 8).unwrap();
        let f = w.into_inner().into_inner();
        assert_eq!(
            DffReader::new(Cursor::new(&f)).unwrap().channel_mask(),
            mask
        );
    }
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn dsf_writer_takes_its_type_from_a_mask() {
    for (ch, mask, t) in [
        (4, 0x0F, 5),
        (4, 0x33, 4),
        (5, 0x37, 6),
        (6, 0x3F, 7),
        (1, 0x04, 1),
        // No type has these: the type of the count.
        (4, 0x3C, 4),
        (2, 0x30, 2),
        (2, 0x3F, 2),
        (6, 0, 7),
    ] {
        assert_eq!(channel_type_for_mask(mask, ch), t, "{mask:#x}");
        let mut w = DsfWriter::new(Cursor::new(Vec::new()), ch, 2_822_400, None).unwrap();
        w.set_channel_mask(mask);
        w.write_bytes(&chans(ch)).unwrap();
        w.finish(BYTES as u64 * 8).unwrap();
        let f = w.into_inner().into_inner();
        let r = DsfReader::new(Cursor::new(&f)).unwrap();
        assert_eq!(r.channel_type(), Some(t));
        assert_eq!(r.channel_mask(), channel_mask_for_type(t, ch));
    }
    assert_eq!(channel_mask_for_type(0, 1), None);
    assert_eq!(channel_mask_for_type(8, 6), None);
    assert_eq!(channel_mask_for_type(7, 5), None);
}

#[test]
fn wav_channel_mask_reads_back() {
    let write = |format, ch: u16, mask: Option<u32>| {
        let c = Cursor::new(Vec::new());
        let mut w = match mask {
            Some(m) => WavWriter::with_channel_mask(c, format, ch, 48_000, m).unwrap(),
            None => WavWriter::new(c, format, ch, 48_000).unwrap(),
        };
        w.write_raw(&vec![0u8; format.bytes() * usize::from(ch) * 4])
            .unwrap();
        w.finish().unwrap();
        w.into_inner().into_inner()
    };
    let mask = |f: Vec<u8>| WavReader::new(Cursor::new(f)).unwrap().channel_mask();
    // A plain 16-bit stereo file declares no mask.
    assert_eq!(mask(write(WavFormat::I16, 2, None)), None);
    assert_eq!(mask(write(WavFormat::I16, 2, Some(0x03))), None);
    // Extensible files carry theirs.
    assert_eq!(mask(write(WavFormat::I24, 2, None)), Some(0x03));
    assert_eq!(mask(write(WavFormat::I24, 6, None)), Some(0x3F));
    assert_eq!(mask(write(WavFormat::I24, 4, Some(0x0F))), Some(0x0F));
    assert_eq!(mask(write(WavFormat::F32, 3, Some(0))), Some(0));
    // A 16-bit stereo file of other speakers needs the extensible form.
    let f = write(WavFormat::I16, 2, Some(0x30));
    assert_eq!(f.len(), 68 + 16);
    assert_eq!(mask(f), Some(0x30));
    // A mask must name one speaker per channel (or none).
    let bad =
        WavWriter::with_channel_mask(Cursor::new(Vec::new()), WavFormat::I24, 4, 48_000, 0x3F);
    assert!(matches!(bad, Err(dsd2pcm::Error::InvalidArgument(_))));
}
