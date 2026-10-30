[User guide](README.md) › Everyday tasks

# Export and printing

Print a document, save its pages as pictures, or save its text.

## Printing

1. Press Ctrl+P, or **Print** at the right of the commands row (or ☰ > Print…).
2. Choose the printer, the pages (**All pages**, or **Page range**, which starts as the selected pages when there
   are any), the copies, and **Print on** one side or both sides (flipped on the long or the short edge).
3. Press **Print**.

The dialog opens at once and fills in the printers when the system has listed them; until then it says it is
looking for printers, and Cancel closes it. If the printing service doesn't answer within 10 seconds or can't
list the printers, the dialog says why; with no printers set up, it says so. Either way, **Try again** looks once
more (after adding a printer in the system's settings, say).

Printing runs in the background. A page range LimarePDF can't read is reported under it, with what you typed kept.
**Print as image** is under Advanced (see below). To print only some text, select it and right-click > Print
selected text… (see [Reading and finding](reading-and-finding.md#print-or-save-just-the-selection)).

Not yet available: paper size, scaling, colour and printer-specific options. The printer's defaults apply.

**Linux and macOS** print through CUPS. The chosen pages go to `lp` as an unencrypted PDF, which CUPS draws with its
own engine, so a printout can differ from LimarePDF's screen. "Print as image" (`--as-image`) sends pages drawn by
LimarePDF at 300 dpi instead. Each job names the printer's default paper size (read with `lpoptions`), as other apps'
jobs do: some printer setups print every page blurred when a job leaves it out. Form fields are drawn into the pages
sent (those meant for printing), since some CUPS setups drop fields without drawing them; the open document keeps
its form. A job that CUPS doesn't take within 2 minutes, or doesn't confirm within 2 minutes after taking it, is
stopped, with a message saying it may have been sent: check the printer's queue before printing again.

**Windows** prints through the print spooler. LimarePDF draws each page at the printer's resolution, at most 600 dpi
and at most 268 million pixels (A0 is drawn at about 415 dpi, which the printer scales up). A page prints at actual
size, shrunk to the printable area when it is larger, centred, and turned when a landscape page meets portrait
paper or the reverse. When the spooler takes no step for 2 minutes (a printer that doesn't answer, say), LimarePDF
stops waiting and says so, and other background work (Append PDFs, OCR, exports) can go ahead. The job itself
can't be stopped on Windows and may still print once the printer is ready: check the printer's queue before
printing again.

When LimarePDF draws the pages, annotations not flagged for printing are left out.

A filled form field with no drawing of its own is drawn by LimarePDF when printed, however the pages are sent (as a
PDF, as images, or on Windows). On every system, a filled field meant for printing that has no drawing of its own,
and whose text LimarePDF can't draw (Hebrew or Arabic, say, or characters no installed font has, filled in by an
app that left the drawing to the reader), would come out empty. Print names those fields and asks first (Print anyway);
`limarepdf-cli print` names them and sends nothing (it has no way to print anyway: open the file in the app for
that).

On the command line: `limarepdf-cli printers` and `limarepdf-cli print`.

## Pages as images

1. Select the pages to export (or none, for every page), and choose ☰ > **Export pages as images…**, which opens
   **Export pages as images** in the inspector.
2. Check the pages, and choose **PNG** or **JPEG** (and the JPEG quality), and the resolution.
3. Press **Choose folder…** and pick where the files go.
4. Close the tool with the × at the top of the inspector.

Pages are saved one file per page, named after the document and the page number (`report-001.png`, …), at 36 to
1152 dpi. Exporting runs in the background and asks before replacing files. The resolution is stored in each file,
so the images import back at the page's size. LimarePDF draws the pages with its own renderer.

A page image can have at most 268 million pixels (A3 at 1152 dpi; a 4 × 3 foot poster at about 390 dpi); a larger
one is an error that names the page, and a lower resolution works. Large page images are drawn in bands, which
bounds the memory a complex page takes but makes some pages slower to export
([details](rendering.md#large-drawings)).

On the command line: `limarepdf-cli render --format png|jpeg`. Like the app, it draws a filled form field that has
no drawing of its own (some apps leave drawing it to the reader). A field whose text LimarePDF can't draw
(Hebrew or Arabic, say, or characters no installed font has) comes out empty: the images are still written, with a warning naming
those fields.

## Text

☰ > **Export text…** (or the command palette, Ctrl+K) writes every page's text to a `.txt` file, with pages
separated by form feeds, as pdftotext does. On the command line, `limarepdf-cli text` prints it. How the text is
read, and its limits: [How text is read](reading-and-finding.md#how-text-is-read).

Word, Excel and CSV export are not available.

## How this was tested

Export and printing have automated tests. On paper, printing has been tried on Linux, with an HP laser printer
through CUPS: pages, mixed A4 and Letter pages, a turned page, comments and filled form fields print correctly. On
Windows only the print dialog has been tried, and nothing has been printed on macOS. The full record is in
[Testing](developer/testing.md#export-and-printing).

---

[← Scans and OCR](scanning-and-ocr.md) · [User guide](README.md) · [Password protection →](security.md)
