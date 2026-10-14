# Export and printing

## Pages as images

Pages are saved as PNG or JPEG files at 36 to 1152 dpi, one file per page, named after the document and the page
number (`report-001.png`, …). The resolution is stored in each file, so the images import back at the page's size.
MPE draws the pages with its own renderer.

Where: the inspector's Export pages as images section, also reached from the top bar's Export menu. It runs in the
background and asks before replacing files. On the command line: `mpe render --format png|jpeg`.

## Text

Export text writes every page's text to a `.txt` file, with pages separated by form feeds, as pdftotext does. Where:
the top bar's Export menu or the inspector. On the command line, `mpe text` prints it. How the text is read, and its
limits: [Search and text](search-and-text.md#text-extraction).

Word, Excel and CSV export are not available.

## Printing

Print (the top bar's Print button, Ctrl+P, or the inspector) chooses the printer, the pages (the selected pages by
default), copies and two-sided printing, and runs in the background. On the command line: `mpe printers` and
`mpe print`.

**Linux and macOS** print through CUPS. The chosen pages go to `lp` as an unencrypted PDF, which CUPS draws with its
own engine, so a printout can differ from MPE's screen. "Print as image" (`--as-image`) sends pages drawn by MPE at
300 dpi instead.

**Windows** prints through the print spooler. MPE draws each page at the printer's resolution, at most 600 dpi. A page
prints at actual size, shrunk to the printable area when it is larger, centred, and turned when a landscape page meets
portrait paper or the reverse.

When MPE draws the pages, annotations not flagged for printing are left out.

Not yet available: paper size, scaling, colour and printer-specific options. The printer's defaults apply.

## How this was tested

- Export: `crates/mpe-core/tests/export.rs` (sizes, content, re-import, names) and by running the CLI.
- Printing: `crates/mpe-core/tests/print.rs`, with a stand-in for `lp` (its arguments, the document it receives, and
  errors), and against a real CUPS with no printers (error messages). The Windows page placement and rotation are
  unit-tested; the Windows printing code has been compiled but not run.
- Printing has not been tried with a real printer yet.
