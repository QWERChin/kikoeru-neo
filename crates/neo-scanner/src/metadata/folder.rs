//! Covers from the images in a work's own folder, for works that no
//! metadata source gave a cover (book/src/scanning.md, "封面").

use std::io::{Read, Seek, SeekFrom};

use futures_util::StreamExt;
use image::{
    DynamicImage, ImageDecoder, ImageReader, Limits, codecs::jpeg::JpegEncoder,
    imageops::FilterType,
};
use neo_db::{
    Db,
    metadata::{self as stored_meta, FolderImage},
};
use neo_storage::{LocalFile, RawPath, RemoteFile, Source, Storage, StorageError};

use super::store::{CoverDirs, FolderCoverError, store_folder_cover};
use crate::{Observer, Stage, names::work_codes};

/// Larger images are never read. A cover does not need them, and on
/// OpenList every byte comes through the provider.
pub const MAX_FOLDER_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
/// Smaller images are likely logos or icons, so any larger candidate of the
/// same rank wins over them.
const SMALL_IMAGE_BYTES: i64 = 32 * 1024;
/// Stored covers are at most this wide. DLsite's own are 560 pixels.
const MAX_COVER_WIDTH: u32 = 1200;
/// A compressed file of 32 MiB can still claim enormous dimensions, so the
/// decoder refuses images beyond these before it allocates.
const MAX_IMAGE_SIDE: u32 = 16_384;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const COVER_QUALITY: u8 = 90;

/// Names that say the image is the cover. Matched in lowercase.
const COVER_WORDS: &[&str] = &[
    "封面",
    "表紙",
    "ジャケット",
    "jacket",
    "cover",
    "アートワーク",
    "artwork",
    "主視覺",
    "主视觉",
    "メインビジュアル",
    "img_main",
];
/// Names of illustrations, which are usually the cover art too.
const PICTURE_WORDS: &[&str] = &[
    "插图",
    "插圖",
    "插画",
    "插畫",
    "イラスト",
    "illust",
    "缩略图",
    "縮略圖",
    "缩图",
    "縮圖",
    "サムネ",
    "thumbnail",
];
/// Variants and side material, ranked after the other images of the same
/// kind: a 封面差分 is still a cover, just not the first choice.
const ASIDE_WORDS: &[&str] = &[
    "差分",
    "草稿",
    "线稿",
    "線稿",
    "壁纸",
    "壁紙",
    "wallpaper",
    "立绘",
    "立繪",
    "立ち絵",
    "介绍",
    "介紹",
    "紹介",
    "角色",
    "キャラ",
    "音轨",
    "音軌",
    "トラック",
    "列表",
    "一覧",
    "一覽",
    "sample",
    "サンプル",
];
/// What the image decoder reads. Other image kinds are never candidates.
const DECODABLE: &[&str] = &["jpg", "jpeg", "png", "webp"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderCoverSummary {
    pub stored: u64,
    /// Missing, oversized or undecodable images. They are not read again
    /// at the same size while the process runs.
    pub failed: u64,
    /// Works left for the next scan because the storage could not answer.
    pub unavailable: u64,
}

/// How good a cover an image's path suggests, lower being better, or
/// `None` when the image should not be used at all.
fn rank(code: &str, path: &str) -> Option<u8> {
    let (dir, file) = match path.rsplit_once('/') {
        Some((dir, file)) => (dir.rsplit('/').next().unwrap_or(dir), file),
        None => ("", path),
    };
    let (stem, ext) = file.rsplit_once('.')?;
    if !DECODABLE.contains(&ext.to_ascii_lowercase().as_str()) {
        return None;
    }
    // Codes are compared in the form discovery stores, so `RJ0123456` in a
    // file name is the work `RJ123456`. Folders sometimes carry other
    // works' covers as advertisements.
    let codes: Vec<String> = work_codes(stem).collect();
    if codes.iter().any(|c| c != code) {
        return None;
    }
    let own = !codes.is_empty();
    let stem = stem.to_lowercase();
    let dir = dir.to_lowercase();
    let cover = |s: &str| COVER_WORDS.iter().any(|w| s.contains(w)) || has_word(s, "main");
    let picture = |s: &str| PICTURE_WORDS.iter().any(|w| s.contains(w));
    if own && cover(&stem) {
        Some(0)
    } else if cover(&stem) {
        Some(1)
    } else if own {
        Some(2)
    } else if cover(&dir) {
        Some(3)
    } else if picture(&stem) {
        Some(4)
    } else if picture(&dir) {
        Some(5)
    } else {
        None
    }
}

/// `word` as a whole run of ASCII letters, so `main` matches `main.jpg`
/// and `01_main` but not `remain`.
fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphabetic())
        .any(|part| part == word)
}

/// The image of one work to use as its cover. Within a rank, side material
/// comes last, and smaller files come first, because a cover is shown at
/// most a few hundred pixels wide and each byte may come from a provider.
pub fn pick(images: &[FolderImage], failed: impl Fn(&FolderImage) -> bool) -> Option<&FolderImage> {
    images
        .iter()
        .enumerate()
        .filter(|(_, image)| !failed(image))
        .filter(|(_, image)| {
            image
                .size
                .is_none_or(|size| u64::try_from(size).is_ok_and(|s| s <= MAX_FOLDER_IMAGE_BYTES))
        })
        .filter_map(|(order, image)| {
            let rank = rank(&image.code, &image.display_path)?;
            let lower = image.display_path.to_lowercase();
            let aside = ASIDE_WORDS
                .iter()
                .any(|w| lower.rsplit('/').next().unwrap_or(&lower).contains(w));
            let (class, size) = match image.size {
                Some(size) if size >= SMALL_IMAGE_BYTES => (0, size),
                None => (1, 0),
                Some(size) => (2, size),
            };
            let depth = image.display_path.matches('/').count();
            Some(((rank, aside, class, size, depth, order), image))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, image)| image)
}

/// Gives each work of `root_id` that has no cover the best image from its
/// folder. The first storage outage stops the stage, like the metadata
/// stage, and the next scan picks the rest up.
///
/// # Errors
/// Any database error.
pub async fn fill_from_folders(
    db: &Db,
    storage: &dyn Storage,
    dirs: &CoverDirs,
    root_id: &str,
    observer: &dyn Observer,
    now: impl Fn() -> i64,
) -> Result<FolderCoverSummary, sqlx::Error> {
    let images = stored_meta::folder_images(db, root_id).await?;
    let chosen: Vec<FolderImage> = {
        let failures = dirs
            .folder_failures
            .lock()
            .expect("failures mutex poisoned");
        images
            .chunk_by(|a, b| a.work_id == b.work_id)
            .filter_map(|work| pick(work, |i| failures.contains(&(i.track_id, i.size))))
            .cloned()
            .collect()
    };
    let mut summary = FolderCoverSummary::default();
    let total = chosen.len() as u64;
    for (done, image) in chosen.into_iter().enumerate() {
        if observer.is_cancelled() {
            break;
        }
        observer.progress(Stage::Covers, done as u64, total);
        match load(storage, &image).await {
            Ok(jpeg) => match store_folder_cover(db, dirs, image.work_id, &jpeg, now()).await {
                Ok(true) => summary.stored += 1,
                Ok(false) => {}
                Err(FolderCoverError::Db(e)) => return Err(e),
                // The data directory is the trouble, which no other image
                // would get past.
                Err(FolderCoverError::Io(e)) => {
                    tracing::warn!(work = image.work_id, error = %e, "cannot store a folder cover; stopping");
                    summary.unavailable = total - done as u64;
                    break;
                }
            },
            Err(LoadError::Unavailable(e)) => {
                tracing::warn!(work = image.work_id, error = %e, "storage unavailable; stopping folder covers");
                summary.unavailable = total - done as u64;
                break;
            }
            Err(LoadError::Unusable(e)) => {
                tracing::warn!(work = image.work_id, path = %image.display_path, error = %e, "cannot use a folder image as the cover");
                dirs.folder_failures
                    .lock()
                    .expect("failures mutex poisoned")
                    .insert((image.track_id, image.size));
                summary.failed += 1;
            }
        }
    }
    observer.progress(Stage::Covers, total, total);
    Ok(summary)
}

enum LoadError {
    /// Try again at the next scan.
    Unavailable(String),
    /// This file will not work as it is.
    Unusable(String),
}

async fn load(storage: &dyn Storage, image: &FolderImage) -> Result<Vec<u8>, LoadError> {
    let unusable = |e: &dyn std::fmt::Display| LoadError::Unusable(e.to_string());
    let location = RawPath::new(image.location_path.clone()).map_err(|e| unusable(&e))?;
    let rel = RawPath::new(image.rel_path.clone()).map_err(|e| unusable(&e))?;
    let bytes = match storage.source(&location.join_path(&rel)).await {
        Ok(Source::LocalFile(local)) => tokio::task::spawn_blocking(move || read_local(&local))
            .await
            .map_err(|e| unusable(&e))??,
        Ok(Source::Remote(remote)) => read_remote(&remote).await?,
        Err(StorageError::Unavailable(e) | StorageError::Upstream(e)) => {
            return Err(LoadError::Unavailable(e));
        }
        Err(e) => return Err(unusable(&e)),
    };
    tokio::task::spawn_blocking(move || to_jpeg(&bytes))
        .await
        .map_err(|e| unusable(&e))?
        .map_err(|e| unusable(&e))
}

/// A file the backend opened but that cannot be read points at the disk
/// or mount, not the image, so read errors count as an outage.
fn read_local(local: &LocalFile) -> Result<Vec<u8>, LoadError> {
    let too_large = || LoadError::Unusable("the image is too large".to_owned());
    if local.size > MAX_FOLDER_IMAGE_BYTES {
        return Err(too_large());
    }
    let outage = |e: std::io::Error| LoadError::Unavailable(e.to_string());
    // The handle is shared, so read from a copy positioned at the start.
    let mut file = local.file.try_clone().map_err(outage)?;
    file.seek(SeekFrom::Start(0)).map_err(outage)?;
    let mut bytes = Vec::new();
    file.take(MAX_FOLDER_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(outage)?;
    if bytes.len() as u64 > MAX_FOLDER_IMAGE_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

async fn read_remote(remote: &RemoteFile) -> Result<Vec<u8>, LoadError> {
    if remote
        .size
        .is_some_and(|size| size > MAX_FOLDER_IMAGE_BYTES)
    {
        return Err(LoadError::Unusable("the image is too large".to_owned()));
    }
    let response = remote
        .opener
        .open(&http::HeaderMap::new())
        .await
        .map_err(|e| match e {
            // A refused download does not show that the image is unusable.
            StorageError::Unavailable(e) | StorageError::Upstream(e) => LoadError::Unavailable(e),
            e => LoadError::Unusable(e.to_string()),
        })?;
    // A throttled provider says nothing about the image.
    if response.status.is_server_error() || response.status == http::StatusCode::TOO_MANY_REQUESTS {
        return Err(LoadError::Unavailable(format!(
            "upstream answered {}",
            response.status
        )));
    }
    if response.status != http::StatusCode::OK {
        return Err(LoadError::Unusable(format!(
            "upstream answered {}",
            response.status
        )));
    }
    let mut body = response.body;
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| LoadError::Unavailable(e.to_string()))?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_FOLDER_IMAGE_BYTES {
            return Err(LoadError::Unusable("the image is too large".to_owned()));
        }
    }
    Ok(bytes)
}

/// Every stored cover is a JPEG of bounded width, whatever the folder held.
fn to_jpeg(bytes: &[u8]) -> Result<Vec<u8>, image::ImageError> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut budget = limits.clone();
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits);
    // The new JPEG carries no EXIF, so a photo's orientation tag is applied
    // to the pixels here or the cover shows sideways.
    let mut decoder = reader.into_decoder()?;
    // Unlike `decode`, `into_decoder` does not weigh the pixel buffer
    // against `max_alloc`, so that check is made here.
    budget.reserve(decoder.total_bytes())?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let image = if image.width() > MAX_COVER_WIDTH {
        image.resize(MAX_COVER_WIDTH, u32::MAX, FilterType::Triangle)
    } else {
        image
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, COVER_QUALITY).encode_image(&image.to_rgb8())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(id: i64, path: &str, size: Option<i64>) -> FolderImage {
        FolderImage {
            work_id: 1,
            code: "RJ01000001".into(),
            track_id: id,
            location_path: b"RJ01000001".to_vec(),
            rel_path: path.as_bytes().to_vec(),
            display_path: path.into(),
            size,
        }
    }

    fn chosen(images: &[FolderImage]) -> Option<&str> {
        pick(images, |_| false).map(|i| i.display_path.as_str())
    }

    const KB: i64 = 1024;

    #[test]
    fn a_cover_name_beats_an_illustration() {
        let images = [
            image(1, "图片/插图.jpg", Some(900 * KB)),
            image(2, "图片/封面.png", Some(3000 * KB)),
            image(3, "图片/音轨列表.png", Some(200 * KB)),
        ];
        assert_eq!(chosen(&images), Some("图片/封面.png"));
    }

    #[test]
    fn the_works_own_main_image_comes_first() {
        let images = [
            image(1, "封面.jpg", Some(200 * KB)),
            image(2, "extra/RJ01000001_img_main.jpg", Some(300 * KB)),
        ];
        assert_eq!(chosen(&images), Some("extra/RJ01000001_img_main.jpg"));
    }

    #[test]
    fn other_works_covers_are_never_used() {
        let images = [
            image(1, "ads/RJ999998_img_main.webp", Some(30 * KB)),
            image(2, "ads/RJ01999997_img_main.webp", Some(30 * KB)),
        ];
        assert_eq!(chosen(&images), None);
    }

    #[test]
    fn variants_and_side_material_come_after_the_plain_cover() {
        let images = [
            image(1, "图片/封面差分小.jpg", Some(170 * KB)),
            image(2, "图片/封面小.jpg", Some(210 * KB)),
            image(3, "图片/封面.png", Some(3900 * KB)),
        ];
        assert_eq!(
            chosen(&images),
            Some("图片/封面小.jpg"),
            "the smaller plain cover"
        );
    }

    #[test]
    fn a_folder_named_for_the_cover_counts_below_the_file_name() {
        let images = [
            image(1, "特典/封面图像/有LOGO.png", Some(3200 * KB)),
            image(2, "特典/封面图像/无LOGO.jpg", Some(530 * KB)),
            image(3, "特典/手机壁纸.jpg", Some(400 * KB)),
        ];
        assert_eq!(chosen(&images), Some("特典/封面图像/无LOGO.jpg"));
        let images = [image(1, "插图/不含LOGO.jpg", Some(1600 * KB))];
        assert_eq!(chosen(&images), Some("插图/不含LOGO.jpg"));
    }

    #[test]
    fn main_must_be_a_whole_word() {
        assert_eq!(
            chosen(&[image(1, "sales/main.jpg", Some(500 * KB))]),
            Some("sales/main.jpg")
        );
        assert_eq!(chosen(&[image(1, "remain.jpg", Some(500 * KB))]), None);
    }

    #[test]
    fn unnamed_images_tiny_logos_and_huge_files_lose() {
        // Game CG and numbered pages say nothing about being the cover.
        assert_eq!(chosen(&[image(1, "CG/01.png", Some(15_000 * KB))]), None);
        let images = [
            image(1, "封面/logo.png", Some(8 * KB)),
            image(2, "封面/封面.jpg", None),
        ];
        assert_eq!(
            chosen(&images),
            Some("封面/封面.jpg"),
            "unknown size beats a tiny file"
        );
        let huge = i64::try_from(MAX_FOLDER_IMAGE_BYTES).unwrap() + 1;
        assert_eq!(chosen(&[image(1, "封面.png", Some(huge))]), None);
        assert_eq!(
            chosen(&[image(1, "封面.gif", Some(100 * KB))]),
            None,
            "not decodable"
        );
    }

    #[test]
    fn a_failed_image_gives_way_to_the_next() {
        let images = [
            image(1, "封面.jpg", Some(200 * KB)),
            image(2, "插图.jpg", Some(900 * KB)),
        ];
        assert_eq!(
            pick(&images, |i| i.track_id == 1).map(|i| i.track_id),
            Some(2)
        );
    }

    #[test]
    fn codes_are_compared_as_discovery_normalizes_them() {
        let mut short = image(1, "RJ0123456_img_main.jpg", Some(200 * KB));
        short.code = "RJ123456".into();
        assert_eq!(chosen(&[short]), Some("RJ0123456_img_main.jpg"));
        let images = [
            image(1, "封面.jpg", Some(200 * KB)),
            image(2, "rj1000001_img_main.jpg", Some(200 * KB)),
        ];
        assert_eq!(chosen(&images), Some("rj1000001_img_main.jpg"));
    }

    #[test]
    fn any_image_becomes_a_bounded_jpeg() {
        let wide = image::RgbImage::from_pixel(2400, 100, image::Rgb([200, 10, 10]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(wide)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let jpeg = to_jpeg(&png).unwrap();
        let back = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(
            image::guess_format(&jpeg).unwrap(),
            image::ImageFormat::Jpeg
        );
        assert_eq!((back.width(), back.height()), (MAX_COVER_WIDTH, 50));
        assert!(to_jpeg(b"not an image").is_err());
    }

    struct Answers(http::StatusCode);

    #[async_trait::async_trait]
    impl neo_storage::RemoteOpen for Answers {
        async fn open(
            &self,
            _headers: &http::HeaderMap,
        ) -> Result<neo_storage::RemoteResponse, StorageError> {
            Ok(neo_storage::RemoteResponse {
                status: self.0,
                headers: http::HeaderMap::new(),
                body: futures_util::stream::iter(Vec::new()).boxed(),
            })
        }
    }

    async fn remote(status: http::StatusCode) -> Result<Vec<u8>, LoadError> {
        read_remote(&RemoteFile {
            size: Some(10),
            mtime_ms: None,
            opener: std::sync::Arc::new(Answers(status)),
        })
        .await
    }

    #[tokio::test]
    async fn throttling_and_server_errors_are_outages_but_a_missing_file_is_not() {
        use http::StatusCode;
        for status in [StatusCode::TOO_MANY_REQUESTS, StatusCode::BAD_GATEWAY] {
            assert!(
                matches!(remote(status).await, Err(LoadError::Unavailable(_))),
                "{status}"
            );
        }
        assert!(matches!(
            remote(StatusCode::NOT_FOUND).await,
            Err(LoadError::Unusable(_))
        ));
    }

    #[test]
    fn a_local_file_that_cannot_be_read_is_an_outage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("封面.jpg");
        // Opened for writing only, so every read fails like a broken mount.
        let file = std::fs::File::create(&path).unwrap();
        let local = LocalFile {
            file: std::sync::Arc::new(file),
            path,
            size: 10,
            mtime_ms: None,
        };
        assert!(matches!(read_local(&local), Err(LoadError::Unavailable(_))));
    }

    #[test]
    fn the_exif_orientation_is_applied_to_the_pixels() {
        use image::ImageEncoder;
        // Left half red, right half blue, tagged "rotate 90 degrees
        // clockwise" (orientation 6) in a little-endian EXIF block.
        let picture = image::RgbImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let exif = vec![
            0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0,
            0, 0,
        ];
        let mut tagged = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut tagged, 95);
        encoder.set_exif_metadata(exif).unwrap();
        encoder
            .write_image(&picture, 40, 20, image::ExtendedColorType::Rgb8)
            .unwrap();

        let upright = image::load_from_memory(&to_jpeg(&tagged).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(upright.dimensions(), (20, 40));
        // The left column becomes the top row.
        assert!(upright.get_pixel(10, 5)[0] > 200, "red on top");
        assert!(upright.get_pixel(10, 35)[2] > 200, "blue below");
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A PNG whose header claims `width`×`height` RGB but whose data is a
    /// few bytes, so only the header can be what the decoder refuses.
    fn png_claiming(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8], data: &[u8]| {
            png.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
            let body = [kind, data].concat();
            png.extend_from_slice(&body);
            png.extend_from_slice(&crc32(&body).to_be_bytes());
        };
        let header = [
            &width.to_be_bytes()[..],
            &height.to_be_bytes(),
            &[8, 2, 0, 0, 0],
        ]
        .concat();
        chunk(b"IHDR", &header);
        chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
        chunk(b"IEND", &[]);
        png
    }

    #[test]
    fn an_image_too_large_to_hold_is_refused_before_its_buffer_is_made() {
        // Within the side limits, but 3 bytes a pixel is past the budget.
        let width = MAX_IMAGE_SIDE;
        let height = u32::try_from(MAX_DECODE_BYTES / 3 / u64::from(width)).unwrap() + 1;
        assert!(matches!(
            to_jpeg(&png_claiming(width, height)),
            Err(image::ImageError::Limits(_))
        ));
        // The same file within the budget fails on its missing data instead.
        assert!(matches!(
            to_jpeg(&png_claiming(64, 64)),
            Err(image::ImageError::Decoding(_))
        ));
    }

    #[test]
    fn an_image_beyond_the_size_limits_is_refused_before_decoding() {
        let tall = image::GrayImage::new(1, MAX_IMAGE_SIDE + 1);
        let mut png = Vec::new();
        image::DynamicImage::ImageLuma8(tall)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert!(matches!(to_jpeg(&png), Err(image::ImageError::Limits(_))));
    }
}
