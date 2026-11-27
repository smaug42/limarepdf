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
   **Optimise**, to make the file smaller and straighten tilted pages. See
   [Getting good results](#getting-good-results) for tilted pages, small print and faxes.
4. Choose **Pages > Recognise text**, which opens **Recognise text (OCR)** in the inspector, and press
   **Recognise**. The pages look the same, but their text can now be searched, selected and copied.
5. Close the tool with the × at the top of the inspector, and save.

A PDF that is already a scan works the same way: start at step 3. When a document's pages are scans without text,
a notice above the pages says so, with a **Recognise text** button.

To straighten tilted pages, optimise before recognising text: pages with text over them, such as recognised text,
aren't straightened. To straighten a page that was recognised already, remove its recognised text, optimise it,
then recognise it again ([Reading again and removing recognised text](#reading-again-and-removing-recognised-text)).
Recognising text doesn't need straight pages: a tilted page is read as well as a straight one
([Tilted pages](#tilted-pages)).

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
  skipped page's reason. On any page, a picture of text or another part of it can be read as an area
  ([Reading an area](#reading-an-area)).
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
- **Turned pages:** a page scanned on its side or upside down is turned upright, as **Pages > Rotate** turns it,
  and read. The report says which pages were turned; the turn and the text are one undo step. A page with only a
  few words is left as it is: they don't show which way up it is.
- **Tilted pages:** a page scanned tilted by up to 5° is read as it is, as well as a straight one, and stays
  tilted. From 2°, the report says so; see [Tilted pages](#tilted-pages).
- **Not handled:** handwriting, vertical text. Text in a picture on a page with text of its own is read only as
  an area.
- **Reading order:** a table with wide gaps, or a form whose fields are far apart, can come out column by column, so
  copied text may not keep its rows together. Each word is still where it shows, so searching and selecting work.

Where: **Pages > Recognise text**, which opens the Recognise text (OCR) tool in the inspector, for the selected
pages or all. It runs in the background and can be undone. On the command line: `limarepdf-cli ocr`.

### Tilted pages

A page scanned a little crooked, tilted by up to 5°, is read line by line as it is, as well as a straight page. The
page itself isn't straightened: it still shows as it was scanned.

- **Its text is written level.** Each line's recognised text runs straight across the page, at the height where
  the printed line crosses the middle of the page's text. The lines keep their order and spacing, as on a straight
  page, so apps read them in order and a table row by row. Text written sloping with the print, as some other OCR
  programs write it, is read out of order and with words in pieces by apps that read text with Poppler, such as
  Evince, Okular and `pdftotext`.
- **Highlights near the sides of the page sit a little off the print.** Over a sloping line, level text is on the
  print in the middle of the page, a little above it towards one side and a little below it towards the other.
  Searching and selecting highlight a word where its text is, so a word near the left or right of the page is
  highlighted a little above or below it. Tilted 2°, the words at the sides of text 16.5 cm (6.5 inches) wide, as
  on a letter or A4 page, are about 3 mm (8 points) off, most of the height of ordinary print; tilted 1°, about
  1.5 mm.
- **The report says so from 2°**, where that shows. In the app, the report after recognising text names each such
  page ("Page 4: tilted 3.3°.") and says how to straighten it; on the command line, the page's line says
  "page 4: tilted 3.3°, 46 lines, 312 words", and a note says how to straighten it.
- **To straighten a page,** remove its recognised text (**Remove recognised text** in the Recognise text (OCR)
  tool), optimise it with **Straighten tilted pages** on (**Pages > Optimise**), then recognise it again. On the
  command line: `remove-ocr`, `optimize-scans`, then `ocr`. Optimising before recognising text does the same in
  one go. The page and its text are then level, and highlights sit on the words. It hardly changes how well the
  text is read: the test sheet tilted 1–5° read with 0.4–0.7% character errors as it was and 0.2–0.5% optimised
  first, and scanned on a flatbed tilted about 3°, with 0.2–1.3% and 0.4–0.7%
  ([Accuracy and speed](#accuracy-and-speed)).

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

### Reading an area

Part of a page can be read on its own, such as a picture of text or a scanned table in an ordinary document,
whether the page has text of its own or not.

- **What is read:** the text inside the area, as the page shows it, at the same resolution as a whole page. Draw
  the area round whole lines: a word its edge cuts through is read in part, or not at all. Words read where the
  page's own text is are left out, as on a whole page.
- **Recognised text in the area:** recognised lines wholly inside it are replaced with the new read, or kept when
  the new read finds nothing. A recognised line crossing its edge is kept, and isn't read again.
- **Several areas** given on the command line are read in one go and are one undo step. Where they overlap on a
  page, the lines they share are read once. In the app, each box is read on its own, one undo step each.
- **Turned and tilted text:** text in the area on its side or upside down isn't read, and the report says so:
  turn the page so the text reads upright (**Pages > Rotate**), read the area, then turn the page back if wanted.
  Tilted text is read as on a tilted page, and isn't reported.
- **Left as it is,** with the reason given: an area off the page, a page with text whose place LimarePDF can't
  tell, and a page with a picture written inside its content (an inline image) when the area holds recognised text
  to replace.

Where: **Pages > Recognise text ▾ > Recognise text in an area** (also in ☰ > Pages), or **Recognise an area** in
the Recognise text (OCR) tool, turns on a tool: drag a box over the text, and it is read in the background.
A note says how many words were read, with Undo; when none were, a message says why. The tool stays on for more
boxes, until Esc, **Recognise an area** again or another tool. An area chosen while another task runs isn't read:
try again once that ends. For a picture, select it and choose **Recognise text** in the inspector, which reads
the picture's area. On the command line: `limarepdf-cli ocr --area PAGE:X,Y,W,H`, in points from the page's top
left as shown, repeated for more areas.

### Seeing and correcting recognised text

**View > Show > Show recognised text** (or ☰ > View > Show > Show recognised text) draws each recognised line's
text in blue over the print it was read from, the print faded under it. The text is drawn in the font it is
written in, Helvetica, stretched along the line as it is written, so a word read wrongly shows where it is.

- **To correct a line,** double-click it. It opens where it is shown, drawn as it will be written, with the caret
  where you clicked and the word selected. With the arrow tool, selecting a line and choosing **Edit > Edit text**
  opens it too, the caret at its end.
- **↓ and ↑** keep the line and open the recognised line below or above, the caret where it was along the line.
- **Enter, Esc, Tab or clicking away** keeps it, one undo step a line. The line stays invisible and in place,
  stretched over its print as before, and searching finds the new text at once. Clearing a line removes it.
- **A line on its side or upside down** is corrected along it in the same way, drawn turned as it is written; ↓
  and ↑ go on to the lines below and above it as it reads. **A tilted line,** as some other OCR programs write it
  over a tilted scan, opens in a box over the line instead.

With Show recognised text off, recognised text isn't drawn, and a double-click selects a word, as on any page.
`limarepdf-cli objects` lists these lines as "(hidden)".

### Getting good results

- **Tilted pages:** optimising first isn't needed for the text: a tilted page is read as well as a straight one,
  and other apps read its text in order too. Optimise first, with Straighten tilted pages on, for a straight page
  and highlights exactly on the words, worth it from about 2°, where the report says a page is tilted
  ([Tilted pages](#tilted-pages)).
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
| Recognised as they are | 51–69% |
| Optimised first, Automatic | 28–57% |
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

**Tilted and turned pages:** the English page of the test sheet, rendered at 300 dpi and tilted, or turned on its
side or upside down. Its text was read back by LimarePDF, and by Poppler's `pdftotext` as the tables above were,
and scored in character errors. Poppler reads the upright page's text with 5.1%, most of them two lines out of
place, where LimarePDF reads it with 0.5%.

| Page | Read by LimarePDF | Read by Poppler | Optimised first, read by Poppler |
| --- | --- | --- | --- |
| Tilted 0.5° or −0.5° | 0.5% | 3.2–5.3% | 0.2–3.6% |
| Tilted 1° | 0.6% | 3.2% | 0.2% |
| Tilted −1° | 0.5% | 4.8% | 3.5% |
| Tilted 2.5° | 0.5% | 2.1% | 4.8% |
| Tilted 4° | 0.4% | 4.7% | 2.4% |
| Tilted 5° | 0.7% | 5.0% | 2.3% |
| On its side or upside down | 0.5% | 5.1% | 4.4% |
| On its side or upside down, and tilted 2.5° | 0.5–0.6% | 2.2–3.2% | 2.4–4.8% |

Each way, 99% of the words or more were found.

**Flatbed scans of the test sheet:** both pages printed on a laser printer and scanned on a flatbed scanner at
300 dpi in greyscale, with the scanner's own straightening and turning off, scored as above.

| Scan | Read by LimarePDF | Read by Poppler | Optimised first, read by Poppler |
| --- | --- | --- | --- |
| English page, straight | 0.4% | 3.4% | 3.5% |
| English page, tilted 0.6° | 0.5% | 3.5% | 3.8% |
| English page, tilted 3.3° or −3.5° | 0.2% | 2.9% | 2.6–2.7% |
| English page, on its side or upside down | 0.2–0.4% | 2.4–2.9% | 2.3–3.0% |
| English page, upside down and tilted 2.9° | 0.3% | 2.6% | 2.6% |
| Other page, straight or upside down | 1.1–1.2% | 1.1–1.2% | 0.7–0.9% |
| Other page, tilted 2.7° | 1.3% | 1.2% | 0.7% |

Each way, 98% of the words or more were found.

The old book pages and the faxes, turned on their side either way or upside down, were turned upright and read as
they were upright: the book pages within 0.6 points of their character errors, the faxes within 2.3 points of their
words found.

On pages made with ImageMagick, an earlier measure found 0.4–1.7% character errors on clean 300 dpi text, 4–14% on
blurred and noisy pages and 0–1.2% under strong shadows. Real scans read worse than those made-up pages.

Speed: about 2.5 seconds a page, from 0.7 to 3.4, on a 6-core machine, start-up included, and 4 seconds for a
large magazine page. A page upside down, or on its side with its top to the right, takes about a second more: it is
read a second time, turned round. Pages run in parallel.

## How this was tested

Importing, optimising and OCR have automated tests. OCR was measured with `tools/ocr-accuracy.py` on the test
sheet, also tilted and turned, rendered and scanned on a flatbed, public scans with known text (old French, Spanish
and German books, faxes), also turned, and real office, flatbed and phone scans, as above. The app's inspector tools
for these weren't clicked through. The full record is in [Testing](developer/testing.md#scanning-and-ocr).

## Licence of the OCR model

The recognition model (`crates/limarepdf-core/models/text-recognition.onnx`, with its list of characters) is
PaddleOCR's PP-OCRv6 small by PaddlePaddle, licensed [Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0) and
included unmodified. See `crates/limarepdf-core/models/README.md`, shipped as `OCR-MODEL.md` in release archives.
With its inference code, OCR adds about 25 MB to each binary.

---

[← Headers, footers, watermarks and stamps](headers-footers-watermarks.md) · [User guide](README.md) · [Export and printing →](export-and-printing.md)
