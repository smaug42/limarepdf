# Export and printing

## Pages as images

Pages are saved as PNG or JPEG files at 36 to 1152 dpi, one file per page, named after the document and the page
number (`report-001.png`, …). The resolution is stored in each file, so the images import back at the page's size.
LimarePDF draws the pages with its own renderer. A page image can have at most 268 million pixels (A3 at 1152 dpi;
a 4 × 3 foot poster at about 390 dpi); a larger one is an error that names the page, and a lower resolution works.
Large page images are drawn in bands, which bounds the memory a complex page takes but makes some pages slower to
export ([details](rendering.md#large-drawings)).

Where: the inspector's Export pages as images section, also reached from the top bar's Export menu. It runs in the
background and asks before replacing files. On the command line: `limarepdf-cli render --format png|jpeg`.

Known limit of the command line: a filled form field with no drawing of its own (some apps leave drawing it to the
reader) comes out empty in `limarepdf-cli render`'s images. The app draws such fields when it opens a file, so its
exports show them, and printing draws them everywhere. To be fixed.

## Text

Export text writes every page's text to a `.txt` file, with pages separated by form feeds, as pdftotext does. Where:
the top bar's Export menu or the inspector. On the command line, `limarepdf-cli text` prints it. How the text is read, and its
limits: [Search and text](search-and-text.md#text-extraction).

Word, Excel and CSV export are not available.

## Printing

Print (the top bar's Print button, Ctrl+P, or the inspector) chooses the printer, the pages (All pages, or Page
range, which shows a Pages to print field below it and starts as the selected pages when there are any), copies and
Print on (one side, or both sides flipped on the long or short edge), and runs in the background. A page range
LimarePDF can't read is reported under it, with what you typed kept. Print as image is under Advanced. On the command line: `limarepdf-cli printers` and
`limarepdf-cli print`.

**Linux and macOS** print through CUPS. The chosen pages go to `lp` as an unencrypted PDF, which CUPS draws with its
own engine, so a printout can differ from LimarePDF's screen. "Print as image" (`--as-image`) sends pages drawn by LimarePDF at
300 dpi instead. Each job names the printer's default paper size (read with `lpoptions`), as other apps' jobs do:
some printer setups print every page blurred when a job leaves it out. Form fields are drawn into the pages sent
(those meant for printing), since some CUPS setups drop fields without drawing them; the open document keeps its
form.

A filled field with no drawing of its own is drawn by LimarePDF when printed, however the pages are sent (as a PDF,
as images, or on Windows). On every system, a filled field meant for printing that has no drawing of its own, and
whose text LimarePDF can't draw (characters beyond Western European, filled in by an app that left the drawing to the reader), would come out
empty. Print names those fields and asks first (Print anyway); `limarepdf-cli print` names them and sends nothing
(it has no way to print anyway: open the file in the app for that).

**Windows** prints through the print spooler. LimarePDF draws each page at the printer's resolution, at most 600 dpi
and at most 268 million pixels (A0 is drawn at about 415 dpi, which the printer scales up). A page
prints at actual size, shrunk to the printable area when it is larger, centred, and turned when a landscape page meets
portrait paper or the reverse.

When LimarePDF draws the pages, annotations not flagged for printing are left out.

Not yet available: paper size, scaling, colour and printer-specific options. The printer's defaults apply.

## How this was tested

- Export: `crates/limarepdf-core/tests/export.rs` (sizes, content, re-import, names) and by running the CLI.
- Printing: `crates/limarepdf-core/tests/print.rs`, with a stand-in for `lp` (its arguments, the document it receives, and
  errors), and against a real CUPS with no printers (error messages). The Windows page placement and rotation are
  unit-tested.
- On paper on Linux, with an HP LaserJet P1606dn through CUPS (October 2026): pages, mixed A4 and Letter pages, a
  turned page, comments and filled form fields print correctly. Getting there took three fixes, each for a way
  CUPS handles PDFs on that setup that other PDFs sent with `lp` suffer from too: the job must name the paper
  size, or every page prints blurred; comments are added after the page's content without resetting the drawing
  state, so they came out upside down on pages that leave it turned; and form fields are removed without being
  drawn. The last two were reproduced without a printer through CUPS's own `cupsfilter`
  ([Testing](testing.md#acceptance-checks)).
- On Windows only the print dialog has been tried; nothing has been printed on paper. Nothing has been printed on
  macOS.
