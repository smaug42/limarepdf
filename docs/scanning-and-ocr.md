[User guide](README.md) › Everyday tasks

# Scans and OCR

LimarePDF turns scans and photos into PDFs, shrinks and straightens scanned pages, and adds a searchable text layer
with built-in text recognition (OCR). It doesn't drive scanners: scan with your system's own scanning app, and open
the pictures in LimarePDF.

## Turn scans into a searchable PDF

1. Scan the pages with your system's scanning app, or take photos of them, as PNG, JPEG or TIFF files.
2. Choose **Make a PDF from images…** on the start screen (or **Insert > From images**, or ☰ > Make a PDF from
   images…) and pick the files. They open as a new document, one page per picture.
3. Optional: choose **Pages > Optimise**, which opens **Optimise scanned pages** in the inspector, and press
   **Optimise**, to make the file smaller and straighten tilted pages.
4. Choose **Pages > Recognise text**, which opens **Recognise text (OCR)** in the inspector, and press
   **Recognise**. The pages look the same, but their text can now be searched, selected and copied.
5. Save.

A PDF that is already a scan works the same way: start at step 3. When a document's pages are scans without text,
a notice above the pages says so, with a **Recognise text** button.

Optimise before recognising text: pages with text over them, such as recognised text, aren't straightened.

## Importing images

- **Formats:** PNG, JPEG and TIFF, including multi-page, 1-bit and CCITT G4 TIFFs.
- **Page size:** from the image's resolution, or 150 dpi when the file has none.
- **Orientation:** pages are turned upright from the EXIF or TIFF orientation.
- **Quality:** JPEGs are embedded as they are, without recompression.

Where: **Insert > From images** or ☰ > Make a PDF from images… (also on the start screen); Insert images in
**Pages > Insert pages**, the inspector, the right-click menu or ☰ > Pages; or drop images on the window. On the
command line: `limarepdf-cli images` and `limarepdf-cli insert-images`.

## Optimising scanned pages

Works on pages that are one image covering the page.

- **Storage:** black and white is stored as CCITT G4; greyscale and colour as JPEG, downsampled to 200 dpi by
  default. "Automatic" chooses per page.
- **Clean-up:** straightens tilts of up to 5° and removes specks.
- **Supported images:** raw or Flate, greyscale and RGB JPEG, and CCITT G4. CMYK JPEG, JBIG2, JPEG 2000, masked images
  and pages with other content are skipped, with the reason given.
- An image is replaced only when the result is smaller, or was straightened or cleaned.
- Pages with text, such as an OCR layer, are not straightened, so optimise before recognising text.
- Only the pages chosen change. A picture they share with other pages (or other parts of the document) stays as it
  was there, so the file then holds both; the report says when. A picture shared by several chosen pages is
  optimised once, as carefully as the most demanding of them needs: at the resolution of the page showing it
  largest, and not straightened if any of them has text over it. The report lists the other pages as optimised with
  the first.

Where: **Pages > Optimise**, which opens the inspector's Optimise scanned pages section, for the selected pages or
all. It runs in the background and can be undone. On the command line: `limarepdf-cli optimize-scans`.

## Recognising text (OCR)

OCR adds an invisible text layer to pages without text, so they can be searched, selected and copied. The page
looks the same.

- **Languages:** Latin script only.
- **How:** LimarePDF's own image processing finds the words, and the recognition model from the
  [ocrs](https://github.com/robertknight/ocrs) project, built into LimarePDF, reads them. A local (Sauvola) threshold decides
  black and white, so shadows and uneven lighting in phone photos are handled.
- **Text layer:** Helvetica with WinAnsi characters; other characters become `?`.
- **Not handled:** text in pictures, handwriting, vertical text.

Where: **Pages > Recognise text**, which opens the inspector's Recognise text (OCR) section, for the selected pages
or all. It runs in the background and can be undone. On the command line: `limarepdf-cli ocr`.

### Seeing and correcting recognised text

**View > Show > Show recognised text** (or ☰ > View > Show recognised text) outlines each recognised line and shows
its text on hover. With the arrow tool, select a line and choose **Edit > Edit text** to correct its text on the
page; it stays invisible and in place, and clearing it removes the line. `limarepdf-cli objects` lists these lines
as "(hidden)".

### Accuracy and speed

Measured on test pages made with ImageMagick:

| Pages | Character errors |
| --- | --- |
| Clean 300 dpi text, serif, sans and mono | 0.4–1.7% |
| Deliberately blurred, noisy and tilted scans | 4–14%, mostly punctuation |
| Strong shadows | 0–1.2% |

One real phone scan, a photocopied card with textured bands, has been tried: all its body text and headings were
read, plus a few short junk lines from the texture, and a two-column table came out column by column.

Speed: two Letter pages in 2.2 seconds on a 6-core machine, about 5 seconds of CPU time per page; pages run in
parallel.

## How this was tested

Importing, optimising and OCR have automated tests, and were tried by hand on made-up scans (CMYK JPEG and 300 dpi
TIFF among them) and one real scan. The black-and-white or greyscale choice hasn't been tried on real scanner
output yet, and the app's inspector sections for these weren't clicked through. The full record is in
[Testing](developer/testing.md#scanning-and-ocr).

## Licence of the OCR model

The recognition model (`crates/limarepdf-core/models/text-recognition.rten`) is by Robert Knight, licensed
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) and included unmodified. See
`crates/limarepdf-core/models/README.md`, shipped as `OCR-MODEL.md` in release archives. With its inference code, OCR adds
about 14 MB to each binary.

---

[← Headers, footers, watermarks and stamps](headers-footers-watermarks.md) · [User guide](README.md) · [Export and printing →](export-and-printing.md)
