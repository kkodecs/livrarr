#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency embedded matching harvest.
//!
//! blocked-pending-implementation: `extract_epub` and `extract_m4b` are private module
//! functions today. These tests invoke the real public file-extraction seam,
//! `extract_and_reconcile`, and assert the harvested `Extraction` fields.
//! blocked-pending-implementation: this new behavioral target is intentionally not
//! registered here because the task forbids Cargo.toml edits.

use livrarr_domain::MediaType;
use livrarr_matching::{extract_and_reconcile, MatchInput};
use std::fs::File;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn write_u16(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_le_bytes());
}

fn write_u32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_le_bytes());
}

fn push_stored_file(
    out: &mut Vec<u8>,
    central: &mut Vec<(String, u32, u32, u32)>,
    name: &str,
    body: &[u8],
) {
    let offset = out.len() as u32;
    let crc = crc32(body);
    write_u32(out, 0x0403_4b50);
    write_u16(out, 20);
    write_u16(out, 0);
    write_u16(out, 0);
    write_u16(out, 0);
    write_u16(out, 0);
    write_u32(out, crc);
    write_u32(out, body.len() as u32);
    write_u32(out, body.len() as u32);
    write_u16(out, name.len() as u16);
    write_u16(out, 0);
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(body);
    central.push((name.to_string(), crc, body.len() as u32, offset));
}

fn write_minimal_epub(path: &Path, opf_identifier: &str) {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    push_stored_file(
        &mut bytes,
        &mut central,
        "mimetype",
        b"application/epub+zip",
    );
    push_stored_file(
        &mut bytes,
        &mut central,
        "META-INF/container.xml",
        br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
    );
    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="bookid" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Dune</dc:title>
    <dc:creator>Frank Herbert</dc:creator>
    <dc:language>en-US</dc:language>
    <dc:identifier id="bookid">ISBN:{opf_identifier}</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx"/>
</package>"#
    );
    push_stored_file(
        &mut bytes,
        &mut central,
        "OEBPS/content.opf",
        opf.as_bytes(),
    );
    // Minimal NCX required by rbook for a valid EPUB parse
    push_stored_file(
        &mut bytes,
        &mut central,
        "OEBPS/toc.ncx",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="bookid"/></head>
  <docTitle><text>Dune</text></docTitle>
  <navMap>
    <navPoint id="np1" playOrder="1">
      <navLabel><text>Start</text></navLabel>
      <content src=""/>
    </navPoint>
  </navMap>
</ncx>"#,
    );

    let central_start = bytes.len() as u32;
    for (name, crc, size, offset) in &central {
        write_u32(&mut bytes, 0x0201_4b50);
        write_u16(&mut bytes, 20);
        write_u16(&mut bytes, 20);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u32(&mut bytes, *crc);
        write_u32(&mut bytes, *size);
        write_u32(&mut bytes, *size);
        write_u16(&mut bytes, name.len() as u16);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u16(&mut bytes, 0);
        write_u32(&mut bytes, 0);
        write_u32(&mut bytes, *offset);
        bytes.extend_from_slice(name.as_bytes());
    }
    let central_size = bytes.len() as u32 - central_start;
    write_u32(&mut bytes, 0x0605_4b50);
    write_u16(&mut bytes, 0);
    write_u16(&mut bytes, 0);
    write_u16(&mut bytes, central.len() as u16);
    write_u16(&mut bytes, central.len() as u16);
    write_u32(&mut bytes, central_size);
    write_u32(&mut bytes, central_start);
    write_u16(&mut bytes, 0);

    std::fs::write(path, bytes).expect("write minimal EPUB fixture");
}

fn write_minimal_m4b_with_asin_tag(path: &Path, asin: &str) {
    // Build a minimal but valid MP4 binary (ftyp + moov > mvhd + udta > meta > hdlr + ilst > ----)
    // so that mp4ameta::Tag::read_from_path can parse the ASIN freeform atom.
    fn box_bytes(fourcc: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let size = (8 + content.len()) as u32;
        let mut out = size.to_be_bytes().to_vec();
        out.extend_from_slice(fourcc);
        out.extend_from_slice(content);
        out
    }
    fn full_box(fourcc: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8; 4];
        body.extend_from_slice(content);
        box_bytes(fourcc, &body)
    }

    let mean = full_box(b"mean", b"com.apple.iTunes");
    let name = full_box(b"name", b"ASIN");
    let mut data_body = vec![0u8, 0, 0, 1, 0, 0, 0, 0]; // type=UTF-8, locale=0
    data_body.extend_from_slice(asin.as_bytes());
    let data = box_bytes(b"data", &data_body);
    let mut ff = Vec::new();
    ff.extend_from_slice(&mean);
    ff.extend_from_slice(&name);
    ff.extend_from_slice(&data);
    let freeform = box_bytes(b"----", &ff);

    // Standard iTunes atoms: ©nam (title) and ©ART (artist) so extract_m4b returns Some
    let title_data = {
        let mut d = vec![0u8, 0, 0, 1, 0, 0, 0, 0];
        d.extend_from_slice(b"Dune");
        box_bytes(b"data", &d)
    };
    let artist_data = {
        let mut d = vec![0u8, 0, 0, 1, 0, 0, 0, 0];
        d.extend_from_slice(b"Frank Herbert");
        box_bytes(b"data", &d)
    };
    let title_atom = box_bytes(b"\xa9nam", &title_data);
    let artist_atom = box_bytes(b"\xa9ART", &artist_data);

    let mut ilst_content = Vec::new();
    ilst_content.extend_from_slice(&title_atom);
    ilst_content.extend_from_slice(&artist_atom);
    ilst_content.extend_from_slice(&freeform);
    let ilst = box_bytes(b"ilst", &ilst_content);

    let mut hdlr_body = vec![0u8; 4]; // version+flags
    hdlr_body.extend_from_slice(&[0u8; 4]); // pre_defined
    hdlr_body.extend_from_slice(b"mdir");
    hdlr_body.extend_from_slice(&[0u8; 12]); // reserved
    hdlr_body.push(0); // null name
    let hdlr = box_bytes(b"hdlr", &hdlr_body);

    let mut meta_body = vec![0u8; 4]; // version+flags (full box)
    meta_body.extend_from_slice(&hdlr);
    meta_body.extend_from_slice(&ilst);
    let meta = box_bytes(b"meta", &meta_body);
    let udta = box_bytes(b"udta", &meta);

    let mut mvhd_body = Vec::new();
    mvhd_body.extend_from_slice(&[0u8; 4]); // version+flags
    mvhd_body.extend_from_slice(&[0u8; 8]); // creation + modification time
    mvhd_body.extend_from_slice(&1000u32.to_be_bytes()); // timescale
    mvhd_body.extend_from_slice(&[0u8; 4]); // duration
    mvhd_body.extend_from_slice(&0x00010000u32.to_be_bytes()); // rate
    mvhd_body.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
    mvhd_body.extend_from_slice(&[0u8; 10]); // reserved
    for &v in &[0x00010000u32, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000u32] {
        mvhd_body.extend_from_slice(&v.to_be_bytes());
    }
    mvhd_body.extend_from_slice(&[0u8; 24]); // pre_defined
    mvhd_body.extend_from_slice(&1u32.to_be_bytes()); // next_track_id
    let mvhd = box_bytes(b"mvhd", &mvhd_body);

    let mut moov_body = Vec::new();
    moov_body.extend_from_slice(&mvhd);
    moov_body.extend_from_slice(&udta);
    let moov = box_bytes(b"moov", &moov_body);

    let mut ftyp_body = Vec::new();
    ftyp_body.extend_from_slice(b"M4B "); // major brand
    ftyp_body.extend_from_slice(&[0u8; 4]); // minor version
    ftyp_body.extend_from_slice(b"M4B "); // compat brand
    let ftyp = box_bytes(b"ftyp", &ftyp_body);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&ftyp);
    bytes.extend_from_slice(&moov);
    std::fs::write(path, bytes).expect("write minimal M4B fixture");
}

async fn extract_primary(path: PathBuf, media_type: MediaType) -> livrarr_matching::Extraction {
    let clusters = extract_and_reconcile(&MatchInput {
        file_path: Some(path),
        grouped_paths: None,
        parse_string: None,
        media_type: Some(media_type),
        scan_root: None,
    })
    .await;
    clusters
        .into_iter()
        .next()
        .expect("embedded extractor should produce a cluster")
        .primary
}

/// REQ-IDs: REQ-006
/// AC-IDs: AC-006
/// Directive: extract_epub harvests ISBN from an OPF dc:identifier before title/author search.
#[tokio::test]

async fn test_wcc_matching_ac_006_extract_epub_harvests_isbn_from_opf_dc_identifier() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("dune.epub");
    write_minimal_epub(&path, "9780441013593");

    let extraction = extract_primary(path, MediaType::Ebook).await;

    assert_eq!(extraction.title.as_deref(), Some("Dune"));
    assert_eq!(extraction.author.as_deref(), Some("Frank Herbert"));
    assert_eq!(
        extraction.isbn.as_deref(),
        Some("9780441013593"),
        "AC-006/REQ-006: EPUB OPF dc:identifier ISBN must be harvested into Extraction.isbn"
    );
}

/// REQ-IDs: REQ-004, REQ-006
/// AC-IDs: AC-004, AC-029
/// Directive: extract_m4b harvests an embedded ASIN tag; ISBN-10-shaped ASIN folds to ISBN.
#[tokio::test]

async fn test_wcc_matching_req_006_extract_m4b_harvests_asin_or_isbn10_shaped_asin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let asin_path = temp.path().join("dune.m4b");
    write_minimal_m4b_with_asin_tag(&asin_path, "B000N2HCP6");

    let asin_extraction = extract_primary(asin_path, MediaType::Audiobook).await;
    assert_eq!(
        asin_extraction.asin.as_deref(),
        Some("B000N2HCP6"),
        "REQ-006: M4B embedded ASIN must be harvested into Extraction.asin"
    );

    // Use a separate tempdir so the stem "dune" matches the embedded title,
    // preventing the reconciler from splitting M1 and M2 into separate clusters.
    let temp2 = tempfile::tempdir().expect("tempdir2");
    let isbn_asin_path = temp2.path().join("dune.m4b");
    write_minimal_m4b_with_asin_tag(&isbn_asin_path, "0441013597");
    let isbn_extraction = extract_primary(isbn_asin_path, MediaType::Audiobook).await;
    assert_eq!(
        isbn_extraction.isbn.as_deref(),
        Some("9780441013593"),
        "REQ-004/AC-029: ISBN-10-shaped ASIN must be converted and harvested as ISBN"
    );
    assert!(
        isbn_extraction.asin.is_none(),
        "REQ-004: ISBN-10-shaped ASIN must not remain in Extraction.asin"
    );
}
