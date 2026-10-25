//! A streaming ZIP writer for work downloads.
//!
//! Entries are stored without compression, because audio and video barely
//! compress, and the archive is written front to back so it can go straight
//! into an HTTP response. Sizes and CRCs follow each entry in a data
//! descriptor. Every entry announces ZIP64 sizes, so a work over 4 GB needs
//! no second pass.

use std::{io, time::SystemTime};

use time::OffsetDateTime;
use tokio::io::{AsyncWrite, AsyncWriteExt};

const LOCAL_HEADER: u32 = 0x0403_4b50;
const DATA_DESCRIPTOR: u32 = 0x0807_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const ZIP64_END: u32 = 0x0606_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const END: u32 = 0x0605_4b50;
/// ZIP64 support in the reader is required (APPNOTE 4.4.3).
const VERSION_NEEDED: u16 = 45;
/// Made on Unix, so the external attributes carry a file mode.
const VERSION_MADE_BY: u16 = (3 << 8) | VERSION_NEEDED;
/// Bit 3: sizes and CRC follow the data. Bit 11: names are UTF-8.
const FLAGS: u16 = (1 << 3) | (1 << 11);
const ZIP64_EXTRA: u16 = 0x0001;
const MAX_32: u64 = 0xFFFF_FFFF;
const MAX_16: u64 = 0xFFFF;

struct Finished {
    name: Vec<u8>,
    dos: (u16, u16),
    crc: u32,
    size: u64,
    offset: u64,
}

struct Open {
    name: Vec<u8>,
    dos: (u16, u16),
    hasher: crc32fast::Hasher,
    size: u64,
    offset: u64,
}

pub struct ZipWriter<W> {
    out: W,
    written: u64,
    entries: Vec<Finished>,
    open: Option<Open>,
}

impl<W: AsyncWrite + Unpin> ZipWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            written: 0,
            entries: Vec::new(),
            open: None,
        }
    }

    async fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes).await?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    /// Starts an entry. `name` uses `/` between directories.
    ///
    /// # Errors
    /// An entry is still open, the name is too long, or the writer failed.
    pub async fn start_file(&mut self, name: &str, modified: Option<SystemTime>) -> io::Result<()> {
        if self.open.is_some() {
            return Err(io::Error::other("the previous entry is still open"));
        }
        let name = name.as_bytes().to_vec();
        let name_len = u16::try_from(name.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry name too long"))?;
        let dos = dos_time(modified);
        let mut header = Vec::with_capacity(30 + name.len() + 20);
        header.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
        header.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
        header.extend_from_slice(&FLAGS.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes()); // stored
        header.extend_from_slice(&dos.0.to_le_bytes());
        header.extend_from_slice(&dos.1.to_le_bytes());
        // CRC and sizes are zero here because bit 3 moves them into the
        // data descriptor.
        header.extend_from_slice(&[0; 12]);
        header.extend_from_slice(&name_len.to_le_bytes());
        header.extend_from_slice(&20u16.to_le_bytes());
        header.extend_from_slice(&name);
        // A ZIP64 extra field in the local header tells readers that the
        // data descriptor carries 8-byte sizes.
        header.extend_from_slice(&ZIP64_EXTRA.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(&[0; 16]);
        let offset = self.written;
        self.put(&header).await?;
        self.open = Some(Open {
            name,
            dos,
            hasher: crc32fast::Hasher::new(),
            size: 0,
            offset,
        });
        Ok(())
    }

    /// # Errors
    /// No entry is open, or the writer failed.
    pub async fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let open = self
            .open
            .as_mut()
            .ok_or_else(|| io::Error::other("no entry is open"))?;
        open.hasher.update(data);
        open.size += data.len() as u64;
        self.put(data).await
    }

    /// # Errors
    /// No entry is open, or the writer failed.
    pub async fn finish_file(&mut self) -> io::Result<()> {
        let open = self
            .open
            .take()
            .ok_or_else(|| io::Error::other("no entry is open"))?;
        let crc = open.hasher.finalize();
        let mut descriptor = Vec::with_capacity(24);
        descriptor.extend_from_slice(&DATA_DESCRIPTOR.to_le_bytes());
        descriptor.extend_from_slice(&crc.to_le_bytes());
        descriptor.extend_from_slice(&open.size.to_le_bytes());
        descriptor.extend_from_slice(&open.size.to_le_bytes());
        self.put(&descriptor).await?;
        self.entries.push(Finished {
            name: open.name,
            dos: open.dos,
            crc,
            size: open.size,
            offset: open.offset,
        });
        Ok(())
    }

    /// Writes the central directory and returns the inner writer, flushed.
    ///
    /// # Errors
    /// An entry is still open, or the writer failed.
    pub async fn finish(mut self) -> io::Result<W> {
        if self.open.is_some() {
            return Err(io::Error::other("an entry is still open"));
        }
        let directory_start = self.written;
        let entries = std::mem::take(&mut self.entries);
        for entry in &entries {
            let header = central_header(entry);
            self.put(&header).await?;
        }
        let directory_size = self.written - directory_start;
        let count = entries.len() as u64;
        let tail = end_records(count, directory_start, directory_size, self.written);
        self.put(&tail).await?;
        self.out.flush().await?;
        Ok(self.out)
    }
}

fn central_header(entry: &Finished) -> Vec<u8> {
    // Only values that do not fit in 32 bits move into the ZIP64 field, in
    // the order APPNOTE 4.5.3 fixes: uncompressed, compressed, offset.
    let mut extra = Vec::new();
    let large_size = entry.size >= MAX_32;
    if large_size {
        extra.extend_from_slice(&entry.size.to_le_bytes());
        extra.extend_from_slice(&entry.size.to_le_bytes());
    }
    let large_offset = entry.offset >= MAX_32;
    if large_offset {
        extra.extend_from_slice(&entry.offset.to_le_bytes());
    }
    let size32 = if large_size { MAX_32 } else { entry.size } as u32;
    let offset32 = if large_offset { MAX_32 } else { entry.offset } as u32;
    let mut header = Vec::with_capacity(46 + entry.name.len() + 4 + extra.len());
    header.extend_from_slice(&CENTRAL_HEADER.to_le_bytes());
    header.extend_from_slice(&VERSION_MADE_BY.to_le_bytes());
    header.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
    header.extend_from_slice(&FLAGS.to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes());
    header.extend_from_slice(&entry.dos.0.to_le_bytes());
    header.extend_from_slice(&entry.dos.1.to_le_bytes());
    header.extend_from_slice(&entry.crc.to_le_bytes());
    header.extend_from_slice(&size32.to_le_bytes());
    header.extend_from_slice(&size32.to_le_bytes());
    header.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
    let extra_len = if extra.is_empty() { 0 } else { 4 + extra.len() };
    header.extend_from_slice(&(extra_len as u16).to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes()); // comment
    header.extend_from_slice(&0u16.to_le_bytes()); // disk
    header.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    header.extend_from_slice(&(0o100_644u32 << 16).to_le_bytes());
    header.extend_from_slice(&offset32.to_le_bytes());
    header.extend_from_slice(&entry.name);
    if !extra.is_empty() {
        header.extend_from_slice(&ZIP64_EXTRA.to_le_bytes());
        header.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        header.extend_from_slice(&extra);
    }
    header
}

/// The ZIP64 end records when any value overflows the classic record, then
/// the classic end of central directory record.
fn end_records(
    count: u64,
    directory_start: u64,
    directory_size: u64,
    zip64_end_at: u64,
) -> Vec<u8> {
    let mut tail = Vec::new();
    let needs_zip64 = count >= MAX_16 || directory_start >= MAX_32 || directory_size >= MAX_32;
    if needs_zip64 {
        tail.extend_from_slice(&ZIP64_END.to_le_bytes());
        tail.extend_from_slice(&44u64.to_le_bytes());
        tail.extend_from_slice(&VERSION_MADE_BY.to_le_bytes());
        tail.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&count.to_le_bytes());
        tail.extend_from_slice(&count.to_le_bytes());
        tail.extend_from_slice(&directory_size.to_le_bytes());
        tail.extend_from_slice(&directory_start.to_le_bytes());
        tail.extend_from_slice(&ZIP64_LOCATOR.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&zip64_end_at.to_le_bytes());
        tail.extend_from_slice(&1u32.to_le_bytes());
    }
    let count16 = count.min(MAX_16) as u16;
    let size32 = directory_size.min(MAX_32) as u32;
    let start32 = directory_start.min(MAX_32) as u32;
    tail.extend_from_slice(&END.to_le_bytes());
    tail.extend_from_slice(&0u16.to_le_bytes());
    tail.extend_from_slice(&0u16.to_le_bytes());
    tail.extend_from_slice(&count16.to_le_bytes());
    tail.extend_from_slice(&count16.to_le_bytes());
    tail.extend_from_slice(&size32.to_le_bytes());
    tail.extend_from_slice(&start32.to_le_bytes());
    tail.extend_from_slice(&0u16.to_le_bytes());
    tail
}

/// MS-DOS time and date in UTC. DOS dates start in 1980.
fn dos_time(modified: Option<SystemTime>) -> (u16, u16) {
    // Converted fallibly: `OffsetDateTime::from` panics outside its range,
    // and one odd timestamp must not abort a whole download.
    let seconds = modified
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let Some(time) = seconds
        .and_then(|s| i64::try_from(s).ok())
        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
    else {
        return (0, (1 << 5) | 1);
    };
    if time.year() < 1980 || time.year() > 2107 {
        return (0, (1 << 5) | 1);
    }
    let dos_time = (u16::from(time.hour()) << 11)
        | (u16::from(time.minute()) << 5)
        | (u16::from(time.second()) / 2);
    let dos_date = (((time.year() - 1980) as u16) << 9)
        | (u16::from(u8::from(time.month())) << 5)
        | u16::from(time.day());
    (dos_time, dos_date)
}

#[cfg(test)]
mod tests {
    use std::{io::Read, time::Duration};

    use super::*;

    async fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Vec::new());
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for (name, data) in files {
            zip.start_file(name, Some(when)).await.unwrap();
            // Several writes per entry, as a proxied file arrives in chunks.
            for chunk in data.chunks(3) {
                zip.write(chunk).await.unwrap();
            }
            zip.finish_file().await.unwrap();
        }
        zip.finish().await.unwrap()
    }

    #[tokio::test]
    async fn a_standard_reader_reads_back_every_entry() {
        let bytes = build(&[
            ("mp3/01 トラック.mp3", b"first file"),
            ("empty.txt", b""),
            ("b.wav", &[7u8; 1000]),
        ])
        .await;
        let mut archive = ::zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 3);
        let mut first = archive.by_name("mp3/01 トラック.mp3").unwrap();
        let mut text = String::new();
        first.read_to_string(&mut text).unwrap();
        assert_eq!(text, "first file");
        drop(first);
        let mut wav = Vec::new();
        archive
            .by_name("b.wav")
            .unwrap()
            .read_to_end(&mut wav)
            .unwrap();
        assert_eq!(wav, vec![7u8; 1000]);
        assert_eq!(archive.by_name("empty.txt").unwrap().size(), 0);
    }

    #[tokio::test]
    async fn many_entries_switch_to_zip64_end_records() {
        let mut zip = ZipWriter::new(Vec::new());
        for i in 0..70_000u32 {
            zip.start_file(&format!("{i}"), None).await.unwrap();
            zip.finish_file().await.unwrap();
        }
        let bytes = zip.finish().await.unwrap();
        let archive = ::zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 70_000);
    }

    #[test]
    fn large_sizes_and_offsets_move_into_the_zip64_field() {
        let entry = Finished {
            name: b"big.wav".to_vec(),
            dos: (0, 0),
            crc: 1,
            size: 5_000_000_000,
            offset: 6_000_000_000,
        };
        let header = central_header(&entry);
        assert_eq!(
            &header[20..28],
            &[0xFF; 8],
            "both 32-bit sizes are saturated"
        );
        assert_eq!(
            &header[42..46],
            &[0xFF; 4],
            "the 32-bit offset is saturated"
        );
        let extra = &header[46 + 7..];
        assert_eq!(&extra[..4], &[0x01, 0x00, 24, 0]);
        assert_eq!(
            u64::from_le_bytes(extra[4..12].try_into().unwrap()),
            5_000_000_000
        );
        assert_eq!(
            u64::from_le_bytes(extra[20..28].try_into().unwrap()),
            6_000_000_000
        );

        let small = Finished {
            size: 10,
            offset: 20,
            ..entry
        };
        assert_eq!(
            central_header(&small).len(),
            46 + 7,
            "no extra field when everything fits"
        );
    }

    #[test]
    fn a_large_directory_offset_gets_zip64_end_records() {
        let tail = end_records(3, 5_000_000_000, 300, 5_000_000_300);
        assert_eq!(&tail[..4], &ZIP64_END.to_le_bytes());
        assert_eq!(tail.len(), 56 + 20 + 22);
        assert_eq!(end_records(3, 1000, 300, 1300).len(), 22);
    }

    #[test]
    fn dos_times_clamp_to_1980() {
        assert_eq!(dos_time(None), (0, 33));
        assert_eq!(dos_time(Some(SystemTime::UNIX_EPOCH)), (0, 33));
        let far = SystemTime::UNIX_EPOCH + Duration::from_secs(253_402_300_800);
        assert_eq!(dos_time(Some(far)), (0, 33));
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(10);
        assert_eq!(dos_time(Some(before)), (0, 33));
        // 2023-11-14 22:13:20 UTC.
        let (t, d) = dos_time(Some(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        ));
        assert_eq!((t >> 11, (t >> 5) & 63, (t & 31) * 2), (22, 13, 20));
        assert_eq!(((d >> 9) + 1980, (d >> 5) & 15, d & 31), (2023, 11, 14));
    }

    #[tokio::test]
    async fn misuse_is_an_error_not_a_corrupt_archive() {
        let mut zip = ZipWriter::new(Vec::new());
        assert!(zip.write(b"x").await.is_err());
        zip.start_file("a", None).await.unwrap();
        assert!(zip.start_file("b", None).await.is_err());
        assert!(zip.finish().await.is_err());
    }
}
