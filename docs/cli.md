[User guide](README.md) › Reference

# Command-line reference

`limarepdf-cli` does most of what the desktop app does to a document, for scripts and batches. Editing page content,
adding comments, and Fill and sign are in the app only: from the command line, page objects can be listed
(`objects`), and comments listed, removed and flattened (`annot`). Every command prints its own options with
`limarepdf-cli <command> --help`.

Commands that change a document take the input file first and write the result to `-o out.pdf`; the input is
never changed: an `-o` that names the input, by any path or link to it, is refused with nothing written (on
Windows a hard link to the input isn't recognised, and gets a file of its own instead). The result is written
whole or not at all: it goes to a new file that is then put in place in one step, so a name that is a hard link
to another file gets a file of its own, and the other file keeps its bytes. Add `--password` to open a protected input. A damaged input is repaired to read it, as in the app,
and a note on stderr says so; some of its content may be missing ([details](rendering.md#damaged-files)). An input
other PDF apps may show differently gets a note on stderr too
([details](rendering.md#files-other-apps-may-show-differently)).

A digitally signed input keeps its signatures: the changes are added after what was signed. When its signatures
don't allow the changes, the command writes nothing unless `--accept-disallowed` is given (the signatures then
report the changes). When the changes can't be added that way (a password set, an encrypted or damaged file,
redaction), it writes nothing unless `--break-signatures` is given, which writes the whole file and breaks them.
`limarepdf-cli sanitise` takes `--break-signatures` too. See
[Saving a signed document](signatures.md#saving-a-signed-document).

Page lists take numbers and ranges, counted from 1: `1-3,7`, `5-` (5 to the end) or `all`.

A command that fails prints `Error:` and the reason on stderr and exits with status 1 (`verify` has its own
statuses, [below](#digital-signatures)). A malformed file that makes LimarePDF fail internally gives "LimarePDF hit
an internal error (...). The file may be damaged."; run the command again with `RUST_BACKTRACE=1` to see where it
happened when reporting it.

When what a command prints can't be written (a full disk, for example), it stops there with an error and status
1; a command that had already saved its file says so ("the file was saved, but its report couldn't be written").

On Linux and macOS a written file is then confirmed on disk, so a power cut can't undo it. When the system can't
confirm it, the file is in place all the same, and the command stops with status 1 and "*file* was saved, but your
system couldn't confirm it is safely on disk (...). If the computer lost power just now, run the command again."
Windows has no way to confirm this: there a power cut in the moment after a file is put in place can still lose it.
When whatever reads the output stops early, as `limarepdf-cli text file.pdf | head` does, the command stops quietly
with the status it would have had otherwise.

## Look at a document

| Command | What it does |
| --- | --- |
| `limarepdf-cli info file.pdf` | Page count, page sizes and document information |
| `limarepdf-cli text in.pdf --pages 1-2` | Print the text of pages |
| `limarepdf-cli search in.pdf "invoice"` | Find text: one line a match, `page 3: …the words around it…`, then the count ("1 match", "12 matches", "No matches"); `--boxes` also prints where each match is on its page (`page 3 [x0 y0 x1 y1]`, points from the top left), a phrase running on to the next page on both |
| `limarepdf-cli objects in.pdf --pages 1` | List the editable objects (text lines, images, paths) with their boxes; OCR text is marked "(hidden)", and a line in more than one font, size or colour shows its first part's, marked "mixed" |
| `limarepdf-cli render in.pdf --dpi 150 --out-dir pngs` | Save pages as images, as the app shows them; `--format jpeg --quality 85` for JPEG. Images already there aren't replaced, and nothing is written, unless you add `--overwrite`. Filled form fields are drawn as the app draws them; a warning names any it can't draw, which come out empty ([details](export-and-printing.md#pages-as-images)) |
| `limarepdf-cli annot list in.pdf` | List the annotations (comments, notes, highlights, drawings): number, page, type, author, date and comment, and which one a reply answers. `--pages 2-3` for some pages; links and form fields are left out |
| `limarepdf-cli annot list in.pdf --json` | The same as a JSON array, with each annotation's colour, opacity and box on the page (points from the top left) |
| `limarepdf-cli form list in.pdf` | List the form fields: name, kind, value, pages, and whether each is required or read-only, with a choice's options |
| `limarepdf-cli form list in.pdf --json` | The fields' values as a JSON object keyed by field name, in the form's order: text as strings, checkboxes as `true`/`false`, a radio group or drop-down's choice as a string (`null` for none), a multi-select list as an array. Push buttons and signatures are left out |

## Forms

| Command | What it does |
| --- | --- |
| `limarepdf-cli form fill in.pdf -o out.pdf --set name=Ann --set agree=on` | Fill in fields. Checkboxes take `on` or `off`; a multi-select list takes its choices separated by `\|` |
| `limarepdf-cli form fill in.pdf -o out.pdf --values values.json` | Fill in fields from a JSON object in the form `limarepdf-cli form list --json` prints, so you can list, edit and fill. A choice can be given by its export value or by the text shown for it. `--set` can be added and is applied after the file |
| `limarepdf-cli form reset in.pdf -o out.pdf` | Put every field back to its default value |
| `limarepdf-cli form flatten in.pdf -o out.pdf` | Draw the fields into the pages as they look now and remove them, so they can no longer be filled in. `--pages 1-3` flattens only those pages; fields elsewhere stay fillable. Refuses, naming the fields, when a shown value has no drawing and LimarePDF can't draw it (text beyond Latin, Greek and Cyrillic), rather than lose it |
| `limarepdf-cli form fill in.pdf -o out.pdf --values values.json --flatten` | Fill in, then flatten |

Filling checks every value first: an unknown field name, a read-only field, text too long for the field, a line
break in a single-line field, or an option the field doesn't have stops it before anything is written. LimarePDF draws
each filled field itself, so the values show in every reader. Field text must be Latin, Greek or Cyrillic for
now; other characters are refused rather than written wrongly. When the field's own font can't show the text, LimarePDF
uses the closest standard font (Helvetica, Times or Courier), or for characters those lack, its Liberation face,
embedded in the file; the field keeps naming its own font, which other apps use when they draw it themselves. Password fields are drawn as `*` and their value is
not kept in the file, as the PDF standard asks. Forms that need JavaScript to calculate or check values are filled,
but their scripts don't run.

Flattening keeps each field's look exactly, including checkboxes and borders; hidden fields are removed without
being drawn. Once no fields are left the form itself is removed, so readers no longer offer to fill it in.

Forms made only for Adobe Reader (XFA forms, whose pages say "Please wait..." elsewhere) have no fields LimarePDF can
reach: `form list` says so (with `--json` it prints `{}` and the reason on standard error), and `form fill` stops
with that reason. Hybrid forms, which carry ordinary fields alongside the XFA, are filled as usual.

## Annotations

| Command | What it does |
| --- | --- |
| `limarepdf-cli annot remove in.pdf -o out.pdf` | Remove every annotation (links and form fields stay) |
| `limarepdf-cli annot remove in.pdf -o out.pdf --type highlight,note --author Sam --pages 1-3` | Remove only those: of these types, by this author (exact match), on these pages. Any of the three can be left out |
| `limarepdf-cli annot flatten in.pdf -o out.pdf` | Draw the annotations into the pages as they look now and remove them, so they are no longer annotations (they become page content). `--pages 1-3` for some pages only |

The types are `highlight`, `underline`, `strikeout`, `squiggly`, `note`, `textbox`, `square`, `circle`, `line`,
`polygon`, `polyline`, `ink` (freehand drawing) and `stamp`; other types can be named as the file does, for example
`FileAttachment`. Removing an annotation also removes the replies to it. Locked annotations are removed too.

Flattening draws highlights, notes, text boxes, shapes, lines, drawings, stamps and carets; replies and comment
pop-ups are removed without being drawn, and hidden annotations are removed. Links, form fields, file attachments,
sound and video, and redaction marks are kept. Annotations stored without a drawing get LimarePDF's drawing first (see
[How comments are drawn](comments-and-markup.md#how-comments-are-drawn)).

## Pages

| Command | What it does |
| --- | --- |
| `limarepdf-cli extract in.pdf -o out.pdf --pages 1-3,7` | Keep only the listed pages |
| `limarepdf-cli delete in.pdf -o out.pdf --pages 2` | Delete pages |
| `limarepdf-cli remove-blank in.pdf -o out.pdf` | Delete pages that look blank and list them. `--tolerance` is the percent of a page that may differ from its background: 0.05 by default (specks or a page number), 0 for nothing at all |
| `limarepdf-cli rotate in.pdf -o out.pdf --pages all --degrees 90` | Rotate clockwise by 90, 180 or 270 |
| `limarepdf-cli reorder in.pdf -o out.pdf --order 3,1,2` | Put all pages in a new order |
| `limarepdf-cli insert-blank in.pdf -o out.pdf --after 0` | Insert a blank page after page N (0 = at the start) |
| `limarepdf-cli crop in.pdf -o out.pdf --page 1 --rect 50,600,400,780` | Set a page's crop box in points: left, bottom, right, top |
| `limarepdf-cli merge a.pdf b.pdf -o out.pdf` | Join PDFs in the order given. The first file's form stays fillable; a later file with form fields is refused unless `--flatten-forms` is given, which draws its fields onto its pages with their answers (they can no longer be filled in). An answer LimarePDF can't draw stops it, naming the field |
| `limarepdf-cli split big.pdf --every 50 --out-dir parts` | Split into files of N pages: `parts/big-001.pdf`, … |
| `limarepdf-cli split big.pdf --bookmarks --out-dir parts` | Split where the top-level bookmarks go: `parts/big-002 Introduction.pdf`, … Pages before the first bookmark go in a file of their own. `--bookmarks 2` also splits at the bookmarks one level down |

Pages that `delete` removes, or that `extract` and `split` leave out, leave nothing in the output: bookmarks,
named destinations, links, form fields and tags stop referring to them, and tags with content on them (and the
tags above those) lose their alternative text. A form nested more than 32 deep makes them stop with an error.
`extract` and `split` keep the bookmarks to
the pages they keep; `delete` keeps every bookmark, without a destination when its page went. `split` writes each
file as soon as its part is made (several at once, one per core); if one can't be written, the error says how many
files already were. When a file is already at one of the parts' names, `split` writes nothing and lists those
files; add `--overwrite` to replace them. `render` does the same for its images.

### Page labels

Page labels are the page numbers viewers show, such as i, ii, 1, 2 or A-1. They don't change what is printed on
the pages. Labels stay with their pages when pages are deleted, moved or extracted; inserted and appended pages
carry on the numbering of the page before them.

| Command | What it does |
| --- | --- |
| `limarepdf-cli labels list in.pdf` | List the ranges: their pages and first and last labels |
| `limarepdf-cli labels set in.pdf -o out.pdf --from 5 --style decimal --start 1` | Number from page 5 on, up to the next range after it. Styles: `decimal`, `lower-roman`, `upper-roman`, `lower-letters`, `upper-letters`, `none` (the prefix alone); `--prefix "A-"` puts text before each number. Pages before the first range keep plain numbers |
| `limarepdf-cli labels remove in.pdf -o out.pdf` | Remove all page labels |

### Viewer preferences

How viewers that follow them show a document when it opens: its layout, side panel and opening page.

| Command | What it does |
| --- | --- |
| `limarepdf-cli view-prefs list in.pdf` | Show the layout, side panel and opening page, or "not set" |
| `limarepdf-cli view-prefs set in.pdf -o out.pdf --layout facing-cover --panel bookmarks --page 3` | Set any of them; those not given are kept. Layouts: `continuous`, `facing`, `facing-cover` (first page alone), `single`; panels: `pages`, `bookmarks`; `none` removes one (`--page none` too) |
| `limarepdf-cli view-prefs clear in.pdf -o out.pdf` | Remove all three. An opening action that isn't a page (such as a script) is kept |

### Bookmarks

Bookmarks are numbered from 1 in the order `list` shows them, each followed by the bookmarks inside it.

| Command | What it does |
| --- | --- |
| `limarepdf-cli bookmarks list in.pdf` | List the bookmarks, numbered and indented by level, with the page each goes to (`-` for another file or a web address) |
| `limarepdf-cli bookmarks add in.pdf -o out.pdf --title "Results" --page 12` | Add a bookmark to page 12, last at the top level. `--after N` puts it just after bookmark N, at its level; `--under N` puts it last inside bookmark N |
| `limarepdf-cli bookmarks rename in.pdf -o out.pdf 3 --title "Method"` | Rename bookmark 3 |
| `limarepdf-cli bookmarks remove in.pdf -o out.pdf 3` | Remove bookmark 3 and the bookmarks inside it |

## Images and scans

| Command | What it does |
| --- | --- |
| `limarepdf-cli images scan.tif photo.jpg -o out.pdf` | One page per image (PNG, JPEG, TIFF; every page of a TIFF). `--dpi` overrides the resolution stored in the files |
| `limarepdf-cli insert-images in.pdf -o out.pdf --after 2 --images a.png b.jpg` | Insert images as pages after page N |
| `limarepdf-cli optimize-scans in.pdf -o out.pdf --mode auto --max-dpi 200 --quality 75` | Shrink and clean up scanned pages; also `--no-deskew`, `--no-despeckle` |
| `limarepdf-cli ocr in.pdf -o out.pdf --pages all` | Add a searchable text layer to pages without text (Latin script) |

See [Scans and OCR](scanning-and-ocr.md) for what these do.

## Stamps

| Command | What it does |
| --- | --- |
| `limarepdf-cli stamp in.pdf -o out.pdf --kind approved --date --author "Sam Lee"` | Put an Approved stamp, with a name-and-date line, in the top right corner of page 1; kinds are `approved`, `not-approved`, `draft`, `final`, `confidential`, `for-comment` and `void` |
| `limarepdf-cli stamp in.pdf -o out.pdf --text "Paid in full" --color green` | Put a stamp with your own wording (shown in capitals) in the top right corner of page 1; colours are `green`, `red` (the default) and `blue`; Latin, Greek and Cyrillic characters; `--date` and `--author` work as with `--kind` |
| `limarepdf-cli stamp in.pdf -o out.pdf --image seal.png --pages all --at 300,700 --scale 50` | Put a picture stamp on every page, centred 300 points from the left and 700 from the top, at half its own size (longer side 160 points) |
| `limarepdf-cli stamp in.pdf -o out.pdf --pages 3 --at 400,650 --signature "Sam Lee" --date` | Sign page 3 with a typed signature in handwriting, centred at that point, with today's date under it; `--style` picks the handwriting (1 Caveat, the default; 2 Cedarville Cursive; 3 Homemade Apple; 4 Zeyada), `--color blue` for blue ink. It fits 150 by 50 points (`--scale` changes that). Latin characters (Caveat has Cyrillic too); a name a style can't show is refused, naming the styles that can. A picture of your signature works with `--image`. This is not a digital signature: see [Fill and sign](fill-and-sign.md) |
| `limarepdf-cli watermark in.pdf -o out.pdf --text CONFIDENTIAL --opacity 0.3` | Add a text watermark |
| `limarepdf-cli watermark in.pdf -o out.pdf --image logo.png --scale 40 --background` | Add a picture watermark, fitted inside 40% of the page, behind the content |
| `limarepdf-cli header-footer in.pdf -o out.pdf --footer-center "Page {page} of {pages}" --header-right "{date}"` | Add headers and footers; tokens `{page}`, `{pages}`, `{date}`, `{bates}` |
| `limarepdf-cli bates in.pdf -o out.pdf --prefix ACME- --digits 6` | Bates-number the pages |

Watermark, header, footer and Bates text is Helvetica, and stamp wording Helvetica Bold; characters they lack
(Polish, Czech, Vietnamese, Greek, Cyrillic and so on) are written in the matching Liberation face, embedded in
the file. Other scripts (Hebrew, Arabic, Chinese and so on) are refused, with nothing written.

## Redaction

| Command | What it does |
| --- | --- |
| `limarepdf-cli redact in.pdf -o out.pdf --text "Sam Lee" --text 555-0142` | Remove every place the text occurs (case is ignored) and paint it black |
| `limarepdf-cli redact in.pdf -o out.pdf --area 2:72,100,200,30` | Remove everything in an area of page 2: 72 points from the left, 100 from the top, 200 wide and 30 high; repeat `--area` for more |
| `limarepdf-cli redact in.pdf -o out.pdf --text "Sam Lee" --json` | The same, printing what was removed as JSON |
| `limarepdf-cli redact in.pdf -o marked.pdf --text "Sam Lee" --mark-only` | Only mark the places for redaction (red outlines), so they can be checked in LimarePDF or another app; nothing is removed |
| `limarepdf-cli redact marked.pdf -o out.pdf --apply` | Apply the redaction marks in the file (from LimarePDF or another app), and any `--text` or `--area` given too |

The command prints what it removed from each page, then the result of its check: it writes the file only when
nothing readable is left under the areas. It warns when removed text is still found elsewhere in the document. See
[Redaction](redaction.md) for exactly what is removed.

### Removing hidden information

| Command | What it does |
| --- | --- |
| `limarepdf-cli sanitise in.pdf --list` | List what the document has of each kind of hidden information |
| `limarepdf-cli sanitise in.pdf -o out.pdf` | Remove metadata, scripts, attachments, hidden layers and private data |
| `limarepdf-cli sanitise in.pdf -o out.pdf --keep metadata --also comments,bookmarks` | Keep some of those, or remove more: `comments`, `form-values`, `bookmarks`, `outside-crop` |

See [Removing hidden information](redaction.md#removing-hidden-information).

## Digital signatures

| Command | What it does |
| --- | --- |
| `limarepdf-cli verify in.pdf` | Check every digital signature: whether the signed part is unchanged, who signed by their certificate, when, and warnings; what changed after each signature and whether the signer allowed it; also lists signature fields not signed yet |
| `limarepdf-cli verify in.pdf --json` | The same as JSON, for scripts |
| `limarepdf-cli verify in.pdf --signed-version 1 -o signed.pdf` | Save the version signature 1 (numbered as `limarepdf-cli verify` lists them) signed: the file as it was when signed, without later changes. Refused for a broken signature |
| `limarepdf-cli verify in.pdf --require-trusted` | Also exit with status 1 unless the file has signatures and every signer is trusted |
| `limarepdf-cli verify in.pdf --trust-file team.txt` | Judge trust by this trust list instead of your own |
| `limarepdf-cli trust list` | The certificates you trust, with their fingerprints (`--json` for scripts) |
| `limarepdf-cli trust add contract.pdf --signature 1` | Trust the signer of signature 1. Compare the SHA-256 fingerprint with them first |
| `limarepdf-cli trust add contract.pdf --signature 1 --authority --pick 2` | Trust a certificate authority from the certificates signature 1 carries; `--pick` chooses when there are several (they are listed) |
| `limarepdf-cli trust add jane.cer` | Trust a signer from a certificate file: `.cer`, `.crt`, `.pem` (DER or PEM) or `.p7c` (as Acrobat exports); `--pick N` for a file with several |
| `limarepdf-cli trust add company-ca.cer --authority` | Trust a certificate authority: every signer it issued a certificate to, directly or through authorities it allowed to |
| `limarepdf-cli trust remove 2` | Stop trusting entry 2 of `limarepdf-cli trust list`, or the certificate with this SHA-256 fingerprint |

Exit status 0 means every signature checked as valid (or there are none) and nothing changed after any of them
that its signer didn't allow; 1 means at least one is broken, can't be checked or has changes that aren't allowed,
or the file can't be opened. A signer nobody vouches for doesn't change the exit status unless `--require-trusted`
is given. See [Digital signatures](signatures.md) for what each result means.

The trust list is `trust.txt` in LimarePDF's settings folder (`~/.config/limarepdf/` on Linux), shared with the app; every
`limarepdf-cli trust` command takes `--trust-file FILE` to work on another one. Trust settings affect LimarePDF only. See
[Trusting signers](signatures.md#trusting-signers).

### Digital IDs

| Command | What it does |
| --- | --- |
| `limarepdf-cli id create --name "Jane Doe"` | Make a self-signed digital ID (RSA 3072-bit, valid five years) and make it the default. Asks for a password twice (at least 8 characters). `--organisation` and `--email` add to the name signatures show |
| `limarepdf-cli id import work.p12` | Import a digital ID from a `.p12` or `.pfx` file (from Acrobat, Windows, macOS or a certificate authority), asking for its password, which it keeps. `--name` chooses its name in LimarePDF |
| `limarepdf-cli id list` | Your digital IDs: number, name, certificate, key, validity and fingerprint; the default is marked (`--json` for scripts) |
| `limarepdf-cli id export jane-doe -o jane.cer` | Save the ID's certificate (no key), for others to trust your signatures |
| `limarepdf-cli id export jane-doe -o jane.p12` | Save the ID with its key, for another app or as a backup, protected by the ID's password; `--new-password` to choose another |
| `limarepdf-cli id remove 2` | Remove ID 2 of `limarepdf-cli id list` (or by name), after you type `yes`; `--yes` doesn't ask. It can't be undone |

Commands that need a password ask for it without showing it; for scripts, `--password-file FILE` (and
`--new-password-file FILE` for `export`) reads it from the first line of a file instead. Passwords are never taken
on the command line itself. IDs are kept in the `ids` folder of LimarePDF's settings folder; every `limarepdf-cli id` command takes
`--id-folder FOLDER` to work on another one. An ID is chosen by its number in `limarepdf-cli id list`, its name (the file
name, such as `jane-doe`), the name in its certificate, or its SHA-256 fingerprint. See
[Digital IDs](signatures.md#digital-ids).

### Signing

| Command | What it does |
| --- | --- |
| `limarepdf-cli sign in.pdf -o signed.pdf --page 2 --rect 350,600,200,60` | Sign with your default digital ID in a new box on page 2: X,Y,W,H in points from the page's top left as shown |
| `limarepdf-cli sign in.pdf -o signed.pdf --field Approver` | Sign the empty signature field `Approver` (as `limarepdf-cli verify` lists it) |
| `limarepdf-cli sign in.pdf -o signed.pdf --field Approver --reason "Approved" --location Ottawa` | Add your reason and location, shown with the signature |
| `limarepdf-cli sign in.pdf -o signed.pdf --page 1 --rect 50,700,200,60 --certify forms` | Certify the document as its author: `no-changes`, `forms` (others may fill in and sign) or `comments` (and comment). Only before anyone signs |
| `limarepdf-cli sign in.pdf -o signed.pdf --field Approver --id work` | Sign with another ID than the default |
| `limarepdf-cli sign in.pdf -o signed.pdf --field Approver --image signature.png` | Show a picture of your handwritten signature beside the text, instead of your name in handwriting (`--style 1` to `4` picks the handwriting). The paper around the ink is made transparent |
| `limarepdf-cli sign in.pdf -o signed.pdf --field Approver --image sheet.jpg --crop 900,2400,1700,2700` | Use only part of the picture: its left, top, right and bottom edges in the picture's pixels, counted from its top left as it is shown (an image viewer shows them), such as the signature on a photo of a whole sheet |

`limarepdf-cli sign` asks for the ID's password (or reads it with `--password-file FILE`) and takes `--id-folder` like
`limarepdf-cli id`. The signature is added after the file's existing bytes, so earlier signatures stay valid, and the signed
copy is checked before it is written; the ID used becomes the default. It refuses expired IDs, encrypted documents,
and what the document's certification or an earlier signer's lock doesn't allow, saying why. See
[Signing](signatures.md#signing).

## Properties and security

| Command | What it does |
| --- | --- |
| `limarepdf-cli set-info in.pdf -o out.pdf --title "Report" --author "Me"` | Set the document's title, author and other properties |
| `limarepdf-cli encrypt in.pdf -o out.pdf --owner-password OWN --user-password USR --cipher aes256 --no-copy` | Protect with passwords; `--no-…` options withdraw permissions |
| `limarepdf-cli decrypt in.pdf -o out.pdf --password USR` | Remove password protection |

See [Password protection](security.md).

## Printing

| Command | What it does |
| --- | --- |
| `limarepdf-cli printers` | List the printers; the default is marked |
| `limarepdf-cli print in.pdf --printer Office --pages 1-3 --copies 2 --sides long-edge` | Print; `--as-image` sends pages drawn by LimarePDF. Refuses, naming the fields and sending nothing, when filled fields meant for printing would come out empty (no drawing, and text LimarePDF can't draw); to print such a file without them, open it in the app and choose Print anyway |

See [Export and printing](export-and-printing.md).

---

[← The desktop app](gui.md) · [User guide](README.md) · [Rendering →](rendering.md)
