# rnb

`rnb` is a command-line program which converts a light novel as an `.epub` into
a custom format that's simpler (and faster) to parse. It's intended to be used
for light novels in Japanese, but it might work for other languages too.

## Building

`rnb` can be built from source by cloning this repository and using Cargo.

```shell
git clone https://github.com/rsookram/rnb
cd rnb
cargo build --release --bin rnb
```

## Usage

```shell
rnb path/to/file.epub
```

Running this will create a new file at `path/to/file.rnb`.

## Supported features

- text for the content of the book
- 振仮名
- images

## Unsupported / partially supported features

- text direction
  - the text direction used for display is determined by the program which
    displays the file instead
- bold text
  - an entire paragraph being bold is supported in some cases
- different font sizes
  - an entire paragraph using a larger font size is supported in some cases
- links to other parts of the book
  - these are usually seen in the table of contents of a book
  - the text that is linked will be included, but information on the target of
    the link won't be preserved
- 外字
  - If a `gaiji.json` file is present in the `.epub`, it will be used to
  replace any 外字 with the corresponding text

### `gaiji.json`

When present in the input `.epub`, this file contains a mapping from filenames
to text that will be used to replace the image that was intended to be rendered
inline. An example looks like:

```json
{
  "gaiji-0.png": "><",
  "gaiji-1.jpg": "㎖"
}
```

In this example, when text like the following is encountered

```html
<p>　参加賞として５００<img class="gaiji" src="../images/gaiji-1.jpg"/>の水が１本<ruby><rb>貰</rb><rt>もら</rt></ruby>える。</p>
```

... It will be included in the output file as:

```
　参加賞として５００㎖の水が１本貰える。
```

(the 振仮名 is stored separately)
