# Export and printing

## Pages as images

Pages are saved as PNG or JPEG files at 36 to 1152 dpi, one file per page, named after the document and the page
number (`report-001.png`, …). The resolution is stored in each file, so the images import back at the page's size.
LimarePDF draws the pages with its own renderer.

Where: the inspector's Export pages as images section, also reached from the top bar's Export menu. It runs in the
background and asks before replacing files. On the command line: `limarepdf-cli render --format png|jpeg`.

## Text

Export text writes every page's text to a `.txt` file, with pages separated by form feeds, as pdftotext does. Where:
the top bar's Export menu or the inspector. On the command line, `limarepdf-cli text` prints it. How the text is read, and its
limits: [Search and text](search-and-text.md#text-extraction).

Word, Excel and CSV export are not available.

## Printing

Print (the top bar's Print button, Ctrl+P, or the inspector) chooses the printer, the pages (the selected pages by
default), copies and two-sided printing, and runs in the background. On the command line: `limarepdf-cli printers` and
`limarepdf-cli print`.

**Linux and macOS** print through CUPS. The chosen pages go to `lp` as an unencrypted PDF, which CUPS draws with its
own engine, so a printout can differ from LimarePDF's screen. "Print as image" (`--as-image`) sends pages drawn by LimarePDF at
300 dpi instead. Each job names the printer's default paper size (read with `lpoptions`), as other apps' jobs do:
some printer setups print every page blurred when a job leaves it out. Form fields are drawn into the pages sent
(those meant for printing), since some CUPS setups drop fields without drawing them; the open document keeps its
form.

**Windows** prints through the print spooler. LimarePDF draws each page at the printer's resolution, at most 600 dpi. A page
prints at actual size, shrunk to the printable area when it is larger, centred, and turned when a landscape page meets
portrait paper or the reverse.

When LimarePDF draws the pages, annotations not flagged for printing are left out.

Not yet available: paper size, scaling, colour and printer-specific options. The printer's defaults apply.

## How this was tested

- Export: `crates/limarepdf-core/tests/export.rs` (sizes, content, re-import, names) and by running the CLI.
- Printing: `crates/limarepdf-core/tests/print.rs`, with a stand-in for `lp` (its arguments, the document it receives, and
  errors), and against a real CUPS with no printers (error messages). The Windows page placement and rotation are
  unit-tested.
- On Linux, printing to an HP LaserJet P1606dn through CUPS gave blurred, unreadable pages. Other programs' PDFs
  sent with `lp` came out the same, and they printed cleanly once the job named the paper size
  (`lp -o media=Letter`), which LimarePDF now does. Comments then printed upside down on a page whose content
  leaves the drawing turned, until each page sent was made to restore the drawing state at its end, and filled
  form fields printed empty, because CUPS's `pdftopdf` step removed the fields (reproduced with `cupsfilter`
  and the printer's PPD), until LimarePDF drew them in itself. On Windows only the print dialog has been tried; nothing
  has been printed on paper. Nothing has been printed on macOS.
