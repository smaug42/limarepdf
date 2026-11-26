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
   **Optimise**, to make the file smaller and straighten tilted pages. Do this for tilted pages: text recognition
   doesn't straighten them. See [Getting good results](#getting-good-results) for small print and faxes.
4. Choose **Pages > Recognise text**, which opens **Recognise text (OCR)** in the inspector, and press
   **Recognise**. The pages look the same, but their text can now be searched, selected and copied.
5. Close the tool with the × at the top of the inspector, and save.

A PDF that is already a scan works the same way: start at step 3. When a document's pages are scans without text,
a notice above the pages says so, with a **Recognise text** button.

Optimise before recognising text: pages with text over them, such as recognised text, aren't straightened. To
straighten a page that was recognised already, remove its recognised text, optimise it, then recognise it again
([Reading again and removing recognised text](#reading-again-and-removing-recognised-text)).

## Importing images

- **Formats:** PNG, JPEG and TIFF, including multi-page, 1-bit and CCITT G4 TIFFs.
- **Page size:** from the image's resolution, or 150 dpi when the file has none.
- **Orientation:** pages are turned upright from the EXIF or TIFF orientation.
- **Quality:** JPEGs are embedded as they are, without recompression.
- **How many at once:** a picture file may take up to 2 GB once opened (a 300 dpi colour letter
  page takes about 25 MB; a JPEG counts as its file's size). Past that, nothing is added, and the
  message says so. The app counts each file on its own; the command line counts the pictures added
  together, so there, add fewer at a time.

Where: **Insert > From images** or ☰ > Make a PDF from images… (also on the start screen); Insert images in
**Pages > Insert pages**, the inspector, the right-click menu or ☰ > Pages; or drop images on the window. On the
command line: `limarepdf-cli images` and `limarepdf-cli insert-images`.

## Optimising scanned pages

Works on pages that are one image covering the page.

- **Storage:** black and white is stored as CCITT G4; greyscale and colour as JPEG, downsampled to 200 dpi by
  default. The resolution is kept in both directions: the scan is made smaller only until it reaches that resolution
  in whichever direction it has fewest pixels for its size on the page. "Automatic" chooses per page.
- **Clean-up:** straightens tilts of up to 5° and removes specks.
- **Supported images:** raw or Flate greyscale and RGB, greyscale and RGB JPEG, and CCITT G4, including those with a
  colour profile or a calibration (CalGray, CalRGB). CMYK images, JBIG2, JPEG 2000, masked images and pages with
  other content are skipped, with the reason given.
- **Colours:** a scan with a colour profile or a calibration keeps it when it stays colour or stays grey, so its
  colours don't change. Made grey or black and white, it is made from its colours as they show.
- An image is replaced only when the result is smaller, or was straightened or cleaned.
- Pages with text, such as an OCR layer, are not straightened, so optimise before recognising text, or remove the
  recognised text first.
- Only the pages chosen change. A picture they share with other pages (or other parts of the document) stays as it
  was there, so the file then holds both; the report says when. A picture shared by several chosen pages is
  optimised once, as carefully as the most demanding of them needs: at the lowest resolution any of them shows it
  at, in either direction, and not straightened if any of them has text over it. The report lists the other pages as optimised with
  the first.

Where: **Pages > Optimise**, which opens the Optimise scanned pages tool in the inspector, for the selected pages
or all. It runs in the background and can be undone. On the command line: `limarepdf-cli optimize-scans`.

## Recognising text (OCR)

OCR adds an invisible text layer to scans and other pages without text, so they can be searched, selected and
copied. The page looks the same.

- **Which pages:** pages without text, and scans with a little text of their own, such as a page number, a Bates
  number or a stamp: a page with text is read when a picture covers at least half of it. Words read where the page's
  own text is are left out, so nothing is doubled, and searching, selecting and copying take in both. Other pages
  with text are skipped, and so are pages with recognised text unless they are read again. The report gives each
  skipped page's reason.
- **Recognised text:** invisible text, as OCR adds it: LimarePDF's own, and other apps' such as OCRmyPDF's, also
  when it is in a group of its own (a form XObject). A line counts when all of it is invisible. Invisible text that
  cuts out a shape for what is drawn after it (clipping) is the page's own.
- **Languages:** those written in Latin letters, accents included: English, French, Spanish, German, Polish,
  Czech, Hungarian and others, with signs such as €, £, °, © and ¿. A few come out as something like them: curly
  quotes as straight ones, … as three dots, and ť and ď, which fonts often draw with a mark like an apostrophe, as t
  and d'. Text in other scripts, such as Greek, Cyrillic or Chinese, isn't read.
- **How:** LimarePDF's own image processing finds the words, and PaddleOCR's PP-OCRv6 small recognition model,
  built into LimarePDF, reads them. A local (Sauvola) threshold decides black and white, so shadows and uneven
  lighting in phone photos are handled.
- **Text layer:** Helvetica for Western European text; a page with other letters (Polish, Czech or Hungarian ones,
  for example) gets Liberation Sans, embedded.
- **Not handled:** text in pictures, handwriting, vertical text, tilted pages that weren't optimised first.
- **Reading order:** a table with wide gaps, or a form whose fields are far apart, can come out column by column, so
  copied text may not keep its rows together. Each word is still where it shows, so searching and selecting work.

Where: **Pages > Recognise text**, which opens the Recognise text (OCR) tool in the inspector, for the selected
pages or all. It runs in the background and can be undone. On the command line: `limarepdf-cli ocr`.

### Reading again and removing recognised text

- **Read recognised pages again,** a checkbox in the Recognise text (OCR) tool, reads pages with recognised text
  anew, for example after a poor read, and replaces their recognised text with the new read. A page keeps its old
  text when the new read finds none.
- **Remove recognised text,** a button in the same tool, takes the recognised text off the chosen pages. They look
  the same, and their own text stays.
- **To straighten a recognised page:** remove its recognised text, optimise it, then recognise it.
- **Left as it is,** with the reason given: recognised text in a group (a form XObject) that draws other content
  too. Reading again skips those pages.

Each runs in the background and is one undo step. On the command line: `limarepdf-cli ocr --again` and
`limarepdf-cli remove-ocr`.

### Seeing and correcting recognised text

**View > Show > Show recognised text** (or ☰ > View > Show > Show recognised text) outlines each recognised
line and shows its text on hover. With the arrow tool, select a line and choose **Edit > Edit text** to correct
its text on the page; it stays invisible and in place, and clearing it removes the line. `limarepdf-cli objects`
lists these lines as "(hidden)".

### Getting good results

- **Tilted pages:** optimise them first, with Straighten tilted pages on. Recognising text doesn't straighten a
  page, and a page tilted 2.5° lost about a third of its characters when it wasn't optimised first.
- **Small print:** scan at 300 dpi, and before optimising, set Optimise's **Image resolution** to 300 dpi
  (`--max-dpi 300` on the command line). At the 200 dpi default a page of small print lost about a tenth of its
  words.
- **Faxes and other scans under 150 dpi:** in Optimise, set **Store as** to Greyscale (`--mode gray`), or don't
  optimise them. Made black and white at that resolution, as Automatic does with text, they lost about a third of
  the words read, and up to more than half.
- **Ordinary print:** 150 dpi is enough. A clean page read as well at 100 and 150 dpi as at 300.

### Accuracy and speed

Two measures are used. Character errors are the share of the true text's characters misread, missed or added:
lower is better. Words found is the share of the true words read, in any order: higher is better. Tesseract 5.5, a
widely used open-source OCR program, read the same pages with its model for each page's language, for comparison.

**A clean test sheet:** the two-page notice in `tools/ocr-sheet/`, rendered at 300 dpi, and printed on a laser
printer and scanned with a phone's scanning app (at 244 dpi).

| Page | Character errors | Scanned | Tesseract, rendered |
| --- | --- | --- | --- |
| English: a letter with a table, a receipt, an email and a web address | 5.1% | 4.2% | 0.7% |
| French, Spanish, German, Polish, Czech and Hungarian, and signs such as € and ° | 0.7% | 0.9% | 1.9% |

On the English page, 99% of the words were found; most of the character errors are two lines read out of place.
The rest are small slips: x read as X in "3 x 6 m", a space added in "45.00", a letter left out of a word. On the
other page, ¡ was missed and ť and ď were read as t and d'.

**Old books:** ten scanned pages from Wikisource, against their proofread text.

| Pages | Character errors | Tesseract |
| --- | --- | --- |
| Three pages of an 1884 French book | 1.8–3.8% | 1.0–1.5% |
| A page of an 1849 French magazine | 1.1% | 0.8% |
| Three pages of two old Spanish books | 0.9–4.0% | 0.2–1.6% |
| Three pages of an 1895 German journal | 1.1–2.2% | 0.5–0.8% |

Optimising first changed them by a point and a half at most, either way.

**Faxes:** five faxed forms from the FUNSD dataset, greyscale at about 90 dpi, with their fax headers.

| | Words found |
| --- | --- |
| Recognised as they are | 51–72% |
| Optimised first, Automatic | 22–55% |
| Optimised first, Greyscale | 52–67% |
| Tesseract | 56–76% |

**Scanners and phones:** letters, bills and forms scanned by their owner. Their true text isn't known, so they were
scored against Tesseract's reading: the share of its words LimarePDF found too. That says how closely the two agree,
not how right either is.

| Scans | Words found |
| --- | --- |
| Office scanners at 150 dpi | 84–98% |
| Office scanners at 200–300 dpi | 74–93% |
| A flatbed scanner at 300 dpi | 59–99% |
| A flatbed scanner, greyscale at 150 dpi | 37–46% |
| A phone's scanning app, a plain printed page | 90% |
| A phone's scanning app, forms on patterned security backgrounds | 25–64% |

Patterned backgrounds read worst. One phone scan of a photocopied card with textured bands was tried earlier: all
its body text and headings were read, plus a few short junk lines from the texture.

**Tilt:** the English page of the test sheet, turned.

| Tilt | Character errors | Optimised first |
| --- | --- | --- |
| 1° | 3.8% | 0.2% |
| 2.5° | 33.8% | 4.8% |
| 4° | 52.6% | 2.4% |

On pages made with ImageMagick, an earlier measure found 0.4–1.7% character errors on clean 300 dpi text, 4–14% on
blurred and noisy pages and 0–1.2% under strong shadows. Real scans read worse than those made-up pages.

Speed: about 2 seconds a page, from 0.6 to 2.7, on a 6-core machine, start-up included, and 3.3 seconds for a
large magazine page. Pages run in parallel.

## How this was tested

Importing, optimising and OCR have automated tests. OCR was measured with `tools/ocr-accuracy.py` on the test
sheet, public scans with known text (old French, Spanish and German books, faxes) and real office, flatbed and phone
scans, as above. The app's inspector tools for these weren't clicked through. The full record is in
[Testing](developer/testing.md#scanning-and-ocr).

## Licence of the OCR model

The recognition model (`crates/limarepdf-core/models/text-recognition.onnx`, with its list of characters) is
PaddleOCR's PP-OCRv6 small by PaddlePaddle, licensed [Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0) and
included unmodified. See `crates/limarepdf-core/models/README.md`, shipped as `OCR-MODEL.md` in release archives.
With its inference code, OCR adds about 25 MB to each binary.

---

[← Headers, footers, watermarks and stamps](headers-footers-watermarks.md) · [User guide](README.md) · [Export and printing →](export-and-printing.md)
