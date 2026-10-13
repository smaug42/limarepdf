# Scanning and OCR

MPE turns scans and photos into PDFs, shrinks and straightens scanned pages, and adds a searchable text layer with
built-in OCR. It does not drive scanners: scan with your system's own tool and import the images. (The usual scanner
and OCR libraries, SANE and Tesseract, are C/C++, which MPE does not use.)

The usual order is: import the images, optimise the pages, then recognise the text.

## Importing images

- **Formats:** PNG, JPEG and TIFF, including multi-page, 1-bit and CCITT G4 TIFFs.
- **Page size:** from the image's resolution, or 150 dpi when the file has none.
- **Orientation:** pages are turned upright from the EXIF or TIFF orientation.
- **Quality:** JPEGs are embedded as they are, without recompression.

Where: ☰ > File > New from images; Insert images in the inspector, the right-click menu or ☰ > Document; or drop
images on the window. On the command line: `mpe images` and `mpe insert-images`.

## Optimising scanned pages

Works on pages that are one image covering the page.

- **Storage:** black and white is stored as CCITT G4; greyscale and colour as JPEG, downsampled to 200 dpi by
  default. "Automatic" chooses per page.
- **Clean-up:** straightens tilts of up to 5° and removes specks.
- **Supported images:** raw or Flate, greyscale and RGB JPEG, and CCITT G4. CMYK JPEG, JBIG2, JPEG 2000, masked images
  and pages with other content are skipped, with the reason given.
- An image is replaced only when the result is smaller, or was straightened or cleaned.
- Pages with text, such as an OCR layer, are not straightened, so optimise before recognising text.

Where: the inspector's Optimise scanned pages section, for the selected pages or all. It runs in the background and
can be undone. On the command line: `mpe optimize-scans`.

## Recognising text (OCR)

OCR adds an invisible text layer to pages without text, so they can be searched, selected and copied. The page
looks the same.

- **Languages:** Latin script only.
- **How:** MPE's own image processing finds the words, and the recognition model from the
  [ocrs](https://github.com/robertknight/ocrs) project, built into MPE, reads them. A local (Sauvola) threshold decides
  black and white, so shadows and uneven lighting in phone photos are handled.
- **Text layer:** Helvetica with WinAnsi characters; other characters become `?`.
- **Not handled:** text in pictures, handwriting, vertical text.

Where: the inspector's Recognise text (OCR) section, for the selected pages or all. It runs in the background and can
be undone. On the command line: `mpe ocr`.

### Seeing and correcting recognised text

☰ > View > Show recognised text outlines each recognised line and shows its text on hover. With the arrow tool, a line
can be selected and its text corrected in the inspector; it stays invisible and in place, and clearing it removes the
line. `mpe objects` lists these lines as "(hidden)".

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

- Import: `crates/mpe-core/tests/import.rs`, and by hand with ImageMagick-made files, including CMYK JPEG and 300 dpi
  G4 TIFF.
- Optimising: `crates/mpe-core/tests/scan.rs`, and by hand on ImageMagick-made 300 dpi scans (5.3 MB down to 0.5 MB, a
  1.2° tilt corrected). The black-and-white or greyscale choice is a heuristic not yet tried on real scanner output.
- OCR: `crates/mpe-core/tests/ocr.rs`, the measurements above, and one real scan. Correcting recognised text was
  tested in code.
- The GUI's inspector sections for these were not clicked through in testing.

## Licence of the OCR model

The recognition model (`crates/mpe-core/models/text-recognition.rten`) is by Robert Knight, licensed
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) and included unmodified. See
`crates/mpe-core/models/README.md`, shipped as `OCR-MODEL.md` in release archives. With its inference code, OCR adds
about 14 MB to each binary.
