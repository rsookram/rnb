use quick_xml::{events::attributes::Attributes, Reader};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    borrow::Cow,
    char::decode_utf16,
    cmp::Reverse,
    env::args_os,
    fs::File,
    io::{BufReader, Read, Write},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    str,
};
use zip::ZipArchive;

type Archive = ZipArchive<File>;

struct TextFiles {
    file_numbers: Box<[usize]>,
}

#[derive(Debug, Default)]
struct ImageFiles {
    names: Box<[Box<str>]>,
    uncompressed_lengths: Box<[u32]>,
    file_numbers: Box<[usize]>,
}

impl ImageFiles {
    fn index_of(&self, src: &[u8]) -> Option<u8> {
        let src_name = src
            .iter()
            .rposition(|&c| c == b'/')
            .map(|i| &src[i + 1..])
            .unwrap_or(src);

        for (i, name) in self.names.iter().enumerate() {
            if name.as_bytes() == src_name {
                return Some(i.try_into().unwrap());
            }
        }

        None
    }
}

#[derive(Debug, Default)]
struct Gaiji {
    names: Box<[Box<str>]>,
    replacements: Box<[Box<[u16]>]>,
}

impl Gaiji {
    fn mapped(&self, src: &[u8]) -> &[u16] {
        let s = src
            .iter()
            .rposition(|&c| c == b'/')
            .map(|i| &src[i + 1..])
            .unwrap_or(src);

        for (i, n) in self.names.iter().enumerate() {
            if n.as_bytes() == s {
                return self.replacements[i].as_ref();
            }
        }

        &[]
    }
}

#[derive(Debug, Default, PartialEq)]
struct Paragraph {
    /// text is empty when the paragraph is an image
    text: Vec<u16>,
    image_idx: Option<u8>,
    ruby: Vec<Ruby>,
    /// flags indicate paragraph-level formatting information.
    /// Lowest bit is bold.
    /// Second-lowest bit is large text.
    flags: u8,
}

#[derive(Debug)]
enum ContentBlock {
    Text {
        text: Box<[u16]>,
        ruby: Box<[Ruby]>,
        /// flags indicate paragraph-level formatting information.
        /// Lowest bit is bold.
        /// Second-lowest bit is large text.
        flags: u8,
    },
    Image {
        index: u8,
    },
}

#[derive(Debug, PartialEq)]
struct Ruby {
    /// startOffset is the offset into the text of the paragraph where this
    /// `reading` starts.
    start_offset: u16,
    /// length is the number of characters in the paragraph that this reading is
    /// associated with.
    length: u8,
    /// reading is the furigana associated with some text within a paragraph.
    reading: Box<[u16]>,
}

fn main() {
    let path = args_os().nth(1).unwrap();
    let input_path = PathBuf::from(&path);

    let mut z: Archive = ZipArchive::new(File::open(&path).unwrap()).unwrap();

    let text_files = get_text_files(&mut z);
    let image_files = get_image_files(&mut z);
    let gaiji = get_gaiji(&mut z);

    let paragraphs = parse_paragraphs(&input_path, text_files, &image_files, gaiji);
    let blocks = merge_paragraphs(paragraphs);

    let output_path = input_path.with_extension("rnb");
    println!("write to {}", output_path.display());

    let out = File::create(&output_path).unwrap();

    write_file(input_path, out, blocks, image_files);
}

fn get_text_files(z: &mut Archive) -> TextFiles {
    let paths = get_text_paths(z);
    let mut contents = Vec::with_capacity(paths.len());

    for p in &paths {
        let num = z.index_for_name(p).unwrap();
        contents.push(num);
    }

    TextFiles {
        file_numbers: contents.into_boxed_slice(),
    }
}

fn get_text_paths(z: &mut Archive) -> Vec<String> {
    let root_file_path = get_root_file_path(z);

    let root_file = z.by_name(&root_file_path).unwrap();
    let root_file = BufReader::with_capacity(16 * 1024, root_file);

    let root_file_dir = root_file_path
        .rsplit_once('/')
        .map(|(before, _)| before)
        .unwrap_or("");

    let mut reader = Reader::from_reader(root_file);
    let config = reader.config_mut();
    config.expand_empty_elements = true;
    config.check_end_names = false;
    config.trim_markup_names_in_closing_tags = false;

    let mut paths = Vec::with_capacity(8);
    let mut buf = Vec::with_capacity(128);
    loop {
        match reader.read_event_into(&mut buf) {
            // Technically itemref might be needed to do things properly, but this might be good
            // enough for now.
            Ok(quick_xml::events::Event::Start(e)) if e.name().as_ref() == b"item" => {
                let mut is_html = false;
                let mut href = String::new();

                for attr in e.attributes().with_checks(false) {
                    let attr = attr.unwrap();
                    match attr.key.as_ref() {
                        b"media-type" => {
                            is_html = *attr.value == *b"application/xhtml+xml";
                        }
                        b"href" => {
                            href = String::from_utf8(attr.value.into_owned()).unwrap();
                        }
                        _ => {}
                    }
                }

                if is_html {
                    let mut path = root_file_dir.to_string();
                    if !path.is_empty() {
                        path.push('/');
                    }
                    path.push_str(&href);

                    paths.push(path);
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            _ => {}
        }

        buf.clear();
    }

    paths
}

fn get_root_file_path(z: &mut Archive) -> String {
    let container = z.by_name("META-INF/container.xml").unwrap();
    let container = BufReader::with_capacity(256, container);

    let mut reader = Reader::from_reader(container);
    let config = reader.config_mut();
    config.expand_empty_elements = true;
    config.check_end_names = false;
    config.trim_markup_names_in_closing_tags = false;

    let mut buf = Vec::with_capacity(64);
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) if e.name().as_ref() == b"rootfile" => {
                for attr in e.attributes().with_checks(false) {
                    let attr = attr.unwrap();
                    if attr.key.as_ref() == b"full-path" {
                        return String::from_utf8(attr.value.into_owned()).unwrap();
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            _ => {}
        }

        buf.clear();
    }

    panic!("couldn't find full-path");
}

fn get_image_files(z: &mut Archive) -> ImageFiles {
    let mut names = Vec::with_capacity(12);
    let mut uncompressed_lengths = Vec::with_capacity(12);
    let mut file_numbers = Vec::with_capacity(12);

    for i in 0..z.len() {
        let f = z.by_index(i).unwrap();
        let path = f.name();

        let is_image = path.ends_with("jpg") || path.ends_with("jpeg") || path.ends_with("png");
        if !is_image {
            continue;
        }

        let name = f.name();

        let name = name
            .rsplit_once('/')
            .map(|(_, after)| after)
            .unwrap_or_else(|| name);
        names.push(name.to_string().into_boxed_str());

        uncompressed_lengths.push(TryInto::<u32>::try_into(f.size()).unwrap());
        file_numbers.push(i);
    }

    ImageFiles {
        names: names.into_boxed_slice(),
        uncompressed_lengths: uncompressed_lengths.into_boxed_slice(),
        file_numbers: file_numbers.into_boxed_slice(),
    }
}

enum GaijiParseState {
    InName { start: usize },
    InReplacement { start: usize },
    NameNext,
    ReplacementNext,
}

fn get_gaiji(z: &mut Archive) -> Gaiji {
    let Ok(mut f) = z.by_name("gaiji.json") else {
        return Gaiji::default();
    };

    let mut content = String::with_capacity(32);
    f.read_to_string(&mut content).unwrap();

    let mut names = Vec::with_capacity(1);
    let mut replacements = Vec::with_capacity(1);

    let mut state = GaijiParseState::NameNext;
    for (i, ch) in content.char_indices() {
        if ch != '"' {
            continue;
        }

        match state {
            GaijiParseState::InName { start } => {
                names.push(content[start..i].to_string().into_boxed_str());
                state = GaijiParseState::ReplacementNext;
            }
            GaijiParseState::InReplacement { start } => {
                replacements.push(content[start..i].encode_utf16().collect());
                state = GaijiParseState::NameNext;
            }
            GaijiParseState::NameNext => state = GaijiParseState::InName { start: i + 1 },
            GaijiParseState::ReplacementNext => {
                state = GaijiParseState::InReplacement { start: i + 1 }
            }
        }
    }

    Gaiji {
        names: names.into_boxed_slice(),
        replacements: replacements.into_boxed_slice(),
    }
}

fn parse_paragraphs(
    input_path: &Path,
    text_files: TextFiles,
    image_files: &ImageFiles,
    gaiji: Gaiji,
) -> Vec<Paragraph> {
    let input = text_files
        .file_numbers
        .into_iter()
        .enumerate()
        .collect::<Vec<_>>();

    let mut result = input
        .par_iter()
        .map(|&(i, num)| {
            let mut archive = ZipArchive::new(File::open(input_path).unwrap()).unwrap();
            let mut f = archive.by_index(num).unwrap();
            let mut buf = String::with_capacity(f.size().try_into().unwrap());
            f.read_to_string(&mut buf).unwrap();

            (i, parse_text_file(&buf, image_files, &gaiji))
        })
        .collect::<Vec<_>>();

    result.sort_by_key(|&(i, _)| i);

    result
        .into_iter()
        .flat_map(|(_, paragraph)| paragraph)
        .collect()
}

enum ParagraphParseState {
    Content {
        flags: u8,
        data: Vec<u16>,
        ruby: Vec<Ruby>,
    },
    Image {
        image_idx: u8,
    },
    None,
}

enum RubyParseState {
    Reading { start_index: usize },
    None,
}

fn parse_text_file(content: &str, image_files: &ImageFiles, gaiji: &Gaiji) -> Vec<Paragraph> {
    let mut reader = Reader::from_str(content);
    let config = reader.config_mut();
    config.expand_empty_elements = true;
    config.check_end_names = false;
    config.trim_markup_names_in_closing_tags = false;

    let mut paragraphs = Vec::with_capacity(256);

    let mut paragraph = ParagraphParseState::None;
    let mut ruby_parse_state = RubyParseState::None;

    let mut buf = Vec::with_capacity(128);
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                match e.name().as_ref() {
                    b"p" => {
                        let flags = match e.try_get_attribute("class").unwrap() {
                            Some(class) => get_flags(&class.value),
                            None => 0,
                        };
                        paragraph = ParagraphParseState::Content {
                            flags,
                            data: Vec::new(),
                            ruby: Vec::new(),
                        };
                    }
                    b"ruby" | b"rb" => {
                        if let ParagraphParseState::Content { ref data, .. } = paragraph {
                            ruby_parse_state = RubyParseState::Reading {
                                start_index: data.len(),
                            };
                        }
                    }
                    b"rt" => {
                        if let RubyParseState::Reading { start_index } = ruby_parse_state {
                            let raw = reader.read_text(e.name()).unwrap();
                            let encoded_reading = raw.encode_utf16();

                            if let ParagraphParseState::Content {
                                data: ref paragraph_data,
                                ref mut ruby,
                                ..
                            } = paragraph
                            {
                                ruby.push(Ruby {
                                    start_offset: start_index.try_into().unwrap(),
                                    length: (paragraph_data.len() - start_index)
                                        .try_into()
                                        .unwrap(),
                                    reading: encoded_reading.collect(),
                                });

                                ruby_parse_state = RubyParseState::Reading {
                                    start_index: paragraph_data.len(),
                                };
                            }
                        }
                    }
                    b"img" => {
                        match parse_img_src(e.attributes()) {
                            ImgSrc::Gaiji(src) => {
                                let encoded = gaiji.mapped(&src);
                                if encoded.is_empty() {
                                    panic!("failed to find mapping for {src:?}");
                                };

                                // Assume that there are no gaiji in ruby
                                if let ParagraphParseState::Content { ref mut data, .. } = paragraph
                                {
                                    data.extend_from_slice(encoded);
                                }
                            }
                            ImgSrc::Illustration(src) => {
                                let image_idx = image_files.index_of(&src).unwrap_or_else(|| {
                                    panic!("<img> doesn't point to a valid image: {e:?}");
                                });
                                paragraph = ParagraphParseState::Image { image_idx };
                            }
                            ImgSrc::None => panic!("unhandled <img>"),
                        }
                    }
                    b"image" => {
                        // images within a <svg>
                        let image_idx = get_attr(e.attributes(), b"href")
                            .and_then(|href| image_files.index_of(&href))
                            .unwrap_or_else(|| {
                                panic!("<image> doesn't point to a valid image: {e:?}");
                            });
                        paragraph = ParagraphParseState::Image { image_idx };
                    }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Text(e)) => {
                if let ParagraphParseState::Content {
                    data: ref mut paragraph_data,
                    ..
                } = paragraph
                {
                    let encoded = str::from_utf8(&e).unwrap().encode_utf16();

                    paragraph_data.extend(encoded);
                }
            }
            Ok(quick_xml::events::Event::CData(e)) => panic!("unhandled CData {e:?}"),
            Ok(quick_xml::events::Event::End(e)) => match e.local_name().as_ref() {
                b"p" => {
                    match paragraph {
                        ParagraphParseState::Content { flags, data, ruby } => {
                            paragraphs.push(Paragraph {
                                text: data,
                                image_idx: None,
                                ruby,
                                flags,
                            });
                        }
                        ParagraphParseState::Image { image_idx } => paragraphs.push(Paragraph {
                            text: Vec::new(),
                            image_idx: Some(image_idx),
                            ruby: Vec::new(),
                            flags: 0,
                        }),
                        _ => {}
                    }

                    paragraph = ParagraphParseState::None;
                }
                b"ruby" => {
                    ruby_parse_state = RubyParseState::None;
                }
                _ => {}
            },
            Ok(quick_xml::events::Event::Eof) => break,
            _ => {}
        }

        buf.clear();
    }

    // This is mainly for sections which only contain an image, but it would also
    // handle the case where the last <p> isn't closed.
    match paragraph {
        ParagraphParseState::Content { flags, data, ruby } => {
            paragraphs.push(Paragraph {
                text: data,
                image_idx: None,
                ruby,
                flags,
            });
        }
        ParagraphParseState::Image { image_idx } => paragraphs.push(Paragraph {
            text: Vec::new(),
            image_idx: Some(image_idx),
            ruby: Vec::new(),
            flags: 0,
        }),
        _ => {}
    }

    paragraphs
}

enum ImgSrc<'a> {
    Gaiji(Cow<'a, [u8]>),
    Illustration(Cow<'a, [u8]>),
    None,
}

fn parse_img_src(mut attributes: Attributes<'_>) -> ImgSrc<'_> {
    let mut src = Cow::Borrowed(b"".as_slice());
    let mut class = Cow::Borrowed(b"".as_slice());
    for attr in attributes.with_checks(false) {
        let attr = attr.unwrap();
        match attr.key.as_ref() {
            b"src" => src = attr.value,
            b"class" => class = attr.value,
            _ => {}
        }
    }

    if src.is_empty() {
        return ImgSrc::None;
    }

    if class.as_ref() == b"gaiji" {
        ImgSrc::Gaiji(src)
    } else {
        ImgSrc::Illustration(src)
    }
}

fn get_attr<'a>(mut attributes: Attributes<'a>, key: &'static [u8]) -> Option<Cow<'a, [u8]>> {
    for attr in attributes.with_checks(false) {
        let attr = attr.unwrap();
        if attr.key.local_name().as_ref() == key {
            return Some(attr.value);
        }
    }

    None
}

fn get_flags(class: &[u8]) -> u8 {
    let mut flags = 0;

    for name in class.split(|&c| c == b' ') {
        if name == b"bold" {
            flags |= 1 << 0;
            continue;
        }

        if name.starts_with(b"font-1") {
            // This may be an increase in font size in terms of percent or em. For
            // example: font-110per or font-1em30.
            if name.ends_with(b"per") {
                flags |= 1 << 1;
                continue;
            }

            if name.len() > "font-1em".len() {
                flags |= 1 << 1;
            }
        }
    }

    flags
}

fn merge_paragraphs(paragraphs: Vec<Paragraph>) -> Vec<ContentBlock> {
    let mut blocks = Vec::with_capacity(128);

    let mut last_paragraph: Option<Paragraph> = None;
    for paragraph in paragraphs {
        if let Some(index) = paragraph.image_idx {
            if let Some(previous) = last_paragraph.take() {
                blocks.push(ContentBlock::Text {
                    text: previous.text.into_boxed_slice(),
                    ruby: previous.ruby.into_boxed_slice(),
                    flags: previous.flags,
                });
            }

            blocks.push(ContentBlock::Image { index });
            continue;
        }

        // If two adjacent paragraphs have the same formatting, merging them is possible. But these
        // paragraphs are so rare that it's simpler to leave them unmerged.
        if paragraph.flags != 0 {
            if let Some(previous) = last_paragraph.take() {
                blocks.push(ContentBlock::Text {
                    text: previous.text.into_boxed_slice(),
                    ruby: previous.ruby.into_boxed_slice(),
                    flags: previous.flags,
                });
            }

            blocks.push(ContentBlock::Text {
                text: paragraph.text.into_boxed_slice(),
                ruby: paragraph.ruby.into_boxed_slice(),
                flags: paragraph.flags,
            });
            continue;
        }

        let Some(mut previous) = last_paragraph else {
            last_paragraph = Some(paragraph);
            continue;
        };

        // Check if merging would result in a block that's too long.
        // The file format can support longer runs of text, but it's preferable to have text that
        // isn't too long so that all the text in a block can be measured and laid out at once.
        if previous.text.len() + paragraph.text.len() > 127
            || previous.ruby.len() + paragraph.ruby.len() > 127
        {
            blocks.push(ContentBlock::Text {
                text: previous.text.into_boxed_slice(),
                ruby: previous.ruby.into_boxed_slice(),
                flags: previous.flags,
            });

            last_paragraph = Some(paragraph);
            continue;
        }

        previous.text.extend("\n".encode_utf16());

        let new_start_offset: u16 = previous.text.len().try_into().unwrap();
        previous.text.extend(paragraph.text);

        previous
            .ruby
            .extend(paragraph.ruby.into_iter().map(|mut r| {
                r.start_offset += new_start_offset;
                r
            }));

        last_paragraph = Some(previous);
    }

    if let Some(paragraph) = last_paragraph {
        blocks.push(ContentBlock::Text {
            text: paragraph.text.into_boxed_slice(),
            ruby: paragraph.ruby.into_boxed_slice(),
            flags: paragraph.flags,
        });
    }

    blocks
}

// Assuming that there are < 30k blocks per book
// First, metadata on blocks:
// - u16 of number of blocks in the book
//
// Then the image metadata:
// - The number of images (u8)
// - For each image, start offset within entire file (u32), then size of image in bytes
//   (u32)
//
// Then blocks... Block format is:
// - length prefix (u16 for bytes of block text)
//   - the highest three bits are flags; from highest to lowest:
//   - isImage: interpret the remaining bits as an image index
//   - isBold: display the paragraph with bold text
//   - isLarge: display the paragraph with larger text
//
// - text of block (UTF-16LE)
// - list of spans
//   - num furigana spans (u8)
//   - each span has 4 fields
//   - the start offset of where it applies to the text (u16)
//   - the number of chars it applies to in the text (u8)
//   - number of bytes for the reading (u8)
//   - UTF-16LE encoded bytes for reading
//
// After blocks come the image data, one image after the next.
fn write_file(
    input_path: PathBuf,
    mut out: File,
    blocks: Vec<ContentBlock>,
    image_files: ImageFiles,
) {
    let mut buf = Vec::with_capacity(1 << 18);

    let num_blocks: u16 = blocks.len().try_into().unwrap();
    buf.extend_from_slice(&num_blocks.to_le_bytes());

    let image_offsets = image_offsets(&image_files);
    extend_with_image_meta(&mut buf, &image_offsets, &image_files.uncompressed_lengths);

    for block in blocks {
        match block {
            ContentBlock::Text { text, ruby, flags } => {
                let num_text_bytes: u16 = (text.len() * 2).try_into().unwrap();
                if num_text_bytes >= 1 << 13 {
                    panic!(
                        "block text is too long to encode: `{}`",
                        decode_utf16(text)
                            .map(|res| res.unwrap())
                            .collect::<String>(),
                    );
                }

                let mut len_prefix = num_text_bytes;

                // Use the second-highest and third-highest bits for formatting info. 10
                // bits is probably sufficient for block length (in bytes). Proposal
                // is to use two additional bits, leaving 13 bits for the length. 2^13 =
                // 8192 or 4096 chars.
                if flags >= 1 << 2 {
                    // These flags would conflict with the flag for image indices.
                    panic!("invalid paragraph flags: {}", flags);
                }

                len_prefix |= u16::from(flags) << 13;

                buf.extend_from_slice(&len_prefix.to_le_bytes());
                if num_text_bytes == 0 {
                    continue;
                }

                buf.extend(text.iter().flat_map(|ch| ch.to_le_bytes()));

                extend_with_ruby(&mut buf, &ruby);
            }
            ContentBlock::Image { index } => {
                let image_idx_or_len_prefix: u16 = u16::from(index) | (1 << 15);
                buf.extend_from_slice(&image_idx_or_len_prefix.to_le_bytes());
                continue;
            }
        }
    }

    out.write_all(&buf).unwrap();

    write_images(
        &input_path,
        &out,
        &image_files,
        &image_offsets,
        buf.len().try_into().unwrap(),
    );
}

fn extend_with_ruby(buf: &mut Vec<u8>, ruby: &[Ruby]) {
    // Write num furigana spans (u8)
    buf.push(ruby.len().try_into().unwrap());

    for r in ruby {
        buf.extend_from_slice(&r.start_offset.to_le_bytes());

        buf.push(r.length);

        let num_reading_bytes: u8 = (r.reading.len() * 2).try_into().unwrap();
        buf.push(num_reading_bytes);

        buf.extend(r.reading.iter().flat_map(|ch| ch.to_le_bytes()));
    }
}

/// write_images returns the offsets to the start of each image in the output.
fn write_images(
    input_path: &Path,
    out: &File,
    image_files: &ImageFiles,
    image_offsets: &[u32],
    base_offset: u32,
) {
    let mut numbers = (0..image_files.uncompressed_lengths.len()).collect::<Vec<_>>();
    numbers.sort_by_key(|&i| Reverse(image_files.uncompressed_lengths[i]));

    numbers.par_iter().for_each(|&i| {
        let mut archive = ZipArchive::new(File::open(input_path).unwrap()).unwrap();

        let mut f = archive.by_index(image_files.file_numbers[i]).unwrap();

        let mut buf = Vec::with_capacity(image_files.uncompressed_lengths[i] as usize);
        f.read_to_end(&mut buf).unwrap();

        out.write_all_at(&buf, u64::from(image_offsets[i] + base_offset))
            .unwrap();
    });
}

fn image_offsets(image_files: &ImageFiles) -> Box<[u32]> {
    image_files
        .uncompressed_lengths
        .iter()
        .scan(0, |acc, &len| {
            let result = Some(*acc);
            *acc += len;
            result
        })
        .collect::<Box<[_]>>()
}

fn extend_with_image_meta(buf: &mut Vec<u8>, image_offsets: &[u32], uncompressed_lengths: &[u32]) {
    buf.push(image_offsets.len().try_into().unwrap());

    for (i, offset) in image_offsets.iter().enumerate() {
        buf.extend_from_slice(&offset.to_le_bytes());
        buf.extend_from_slice(&uncompressed_lengths[i].to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_paragraph() {
        let content = String::from("<p>test</p>");

        let paragraphs = parse_text_file(&content, &Default::default(), &Default::default());

        assert_eq!(paragraphs.len(), 1);

        let expected = Paragraph {
            text: "test".encode_utf16().collect(),
            ..Default::default()
        };
        assert_eq!(paragraphs[0], expected);
    }

    #[test]
    fn parse_paragraph_ruby() {
        let content = String::from("<p><ruby>開発<rt>かいはつ</rt></ruby></p>");

        let paragraphs = parse_text_file(&content, &Default::default(), &Default::default());

        assert_eq!(paragraphs.len(), 1);

        let expected = Paragraph {
            text: "開発".encode_utf16().collect(),
            ruby: Vec::from([Ruby {
                start_offset: 0,
                length: 2,
                reading: "かいはつ".encode_utf16().collect(),
            }]),
            ..Default::default()
        };
        assert_eq!(paragraphs[0], expected);
    }

    #[test]
    fn parse_paragraph_ruby_rb() {
        let content = String::from("<p><ruby><rb>開発</rb><rt>かいはつ</rt></ruby></p>");

        let paragraphs = parse_text_file(&content, &Default::default(), &Default::default());

        assert_eq!(paragraphs.len(), 1);

        let expected = Paragraph {
            text: "開発".encode_utf16().collect(),
            ruby: Vec::from([Ruby {
                start_offset: 0,
                length: 2,
                reading: "かいはつ".encode_utf16().collect(),
            }]),
            ..Default::default()
        };
        assert_eq!(paragraphs[0], expected);
    }

    #[test]
    fn parse_paragraph_ruby_multiple_rt() {
        let content = String::from("<p><ruby>開<rt>かい</rt>発<rt>はつ</rt></ruby></p>");

        let paragraphs = parse_text_file(&content, &Default::default(), &Default::default());

        assert_eq!(paragraphs.len(), 1);

        let expected = Paragraph {
            text: "開発".encode_utf16().collect(),
            ruby: Vec::from([
                Ruby {
                    start_offset: 0,
                    length: 1,
                    reading: "かい".encode_utf16().collect(),
                },
                Ruby {
                    start_offset: 1,
                    length: 1,
                    reading: "はつ".encode_utf16().collect(),
                },
            ]),
            ..Default::default()
        };
        assert_eq!(paragraphs[0], expected);
    }

    #[test]
    fn merge_single() {
        let paragraph = Paragraph {
            text: "a".encode_utf16().collect(),
            ..Default::default()
        };

        let mut result = merge_paragraphs(vec![paragraph]);

        assert_eq!(result.len(), 1);

        let ContentBlock::Text { text, .. } = result.pop().unwrap() else {
            panic!("image");
        };

        assert_eq!(text, "a".encode_utf16().collect());
    }

    #[test]
    fn merge_two() {
        let paragraph_a = Paragraph {
            text: "a".encode_utf16().collect(),
            ..Default::default()
        };
        let paragraph_b = Paragraph {
            text: "b".encode_utf16().collect(),
            ..Default::default()
        };

        let mut result = merge_paragraphs(vec![paragraph_a, paragraph_b]);

        assert_eq!(result.len(), 1);

        let ContentBlock::Text { text, .. } = result.pop().unwrap() else {
            panic!("image");
        };

        assert_eq!(text, "a\nb".encode_utf16().collect());
    }
}
