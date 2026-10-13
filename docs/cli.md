# Command-line reference

`mpe` does everything the desktop app does to a document, for scripts and batches. Every command prints its own
options with `mpe <command> --help`.

Commands that change a document take the input file first and write the result to `-o out.pdf`; the input is
never changed. Add `--password` to open a protected input.

Page lists take numbers and ranges, counted from 1: `1-3,7`, `5-` (5 to the end) or `all`.

## Look at a document

| Command | What it does |
| --- | --- |
| `mpe info file.pdf` | Page count, page sizes and document information |
| `mpe text in.pdf --pages 1-2` | Print the text of pages |
| `mpe search in.pdf "invoice"` | Find text; `--boxes` also prints where each match is on its page |
| `mpe objects in.pdf --pages 1` | List the editable objects (text lines, images, paths) with their boxes; OCR text is marked "(hidden)" |
| `mpe render in.pdf --dpi 150 --out-dir pngs` | Save pages as images; `--format jpeg --quality 85` for JPEG |

## Pages

| Command | What it does |
| --- | --- |
| `mpe extract in.pdf -o out.pdf --pages 1-3,7` | Keep only the listed pages |
| `mpe delete in.pdf -o out.pdf --pages 2` | Delete pages |
| `mpe rotate in.pdf -o out.pdf --pages all --degrees 90` | Rotate clockwise by 90, 180 or 270 |
| `mpe reorder in.pdf -o out.pdf --order 3,1,2` | Put all pages in a new order |
| `mpe insert-blank in.pdf -o out.pdf --after 0` | Insert a blank page after page N (0 = at the start) |
| `mpe crop in.pdf -o out.pdf --page 1 --rect 50,600,400,780` | Set a page's crop box in points: left, bottom, right, top |
| `mpe merge a.pdf b.pdf -o out.pdf` | Join PDFs in the order given |
| `mpe split big.pdf --every 50 --out-dir parts` | Split into files of N pages: `parts/big-001.pdf`, … |

## Images and scans

| Command | What it does |
| --- | --- |
| `mpe images scan.tif photo.jpg -o out.pdf` | One page per image (PNG, JPEG, TIFF; every page of a TIFF). `--dpi` overrides the resolution stored in the files |
| `mpe insert-images in.pdf -o out.pdf --after 2 --images a.png b.jpg` | Insert images as pages after page N |
| `mpe optimize-scans in.pdf -o out.pdf --mode auto --max-dpi 200 --quality 75` | Shrink and clean up scanned pages; also `--no-deskew`, `--no-despeckle` |
| `mpe ocr in.pdf -o out.pdf --pages all` | Add a searchable text layer to pages without text (Latin script) |

See [Scanning and OCR](scanning-and-ocr.md) for what these do.

## Stamps

| Command | What it does |
| --- | --- |
| `mpe watermark in.pdf -o out.pdf --text CONFIDENTIAL --opacity 0.3` | Add a text watermark |
| `mpe header-footer in.pdf -o out.pdf --footer-center "Page {page} of {pages}" --header-right "{date}"` | Add headers and footers; tokens `{page}`, `{pages}`, `{date}`, `{bates}` |
| `mpe bates in.pdf -o out.pdf --prefix ACME- --digits 6` | Bates-number the pages |

## Properties and security

| Command | What it does |
| --- | --- |
| `mpe set-info in.pdf -o out.pdf --title "Report" --author "Me"` | Set the document's title, author and other properties |
| `mpe encrypt in.pdf -o out.pdf --owner-password OWN --user-password USR --cipher aes256 --no-copy` | Protect with passwords; `--no-…` options withdraw permissions |
| `mpe decrypt in.pdf -o out.pdf --password USR` | Remove password protection |

See [Password security](security.md).

## Printing

| Command | What it does |
| --- | --- |
| `mpe printers` | List the printers; the default is marked |
| `mpe print in.pdf --printer Office --pages 1-3 --copies 2 --sides long-edge` | Print; `--as-image` sends pages drawn by MPE |

See [Export and printing](export-and-printing.md).
