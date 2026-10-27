[User guide](README.md) › Everyday tasks

# Filling in forms

When a document has a form, a notice above the pages says so, and its fields are highlighted.

## Fill in a form

1. Click a field with the read tool (the text cursor). With another tool on, hold Ctrl (⌘ on a Mac) and click.
2. Type, tick or choose.
3. Press Tab for the next field, or Shift+Tab for the one before. Fields go top to bottom, page by page.
   **Forms and sign > Fill fields** also goes to the next field.
4. Save with Ctrl+S, or use Save As (Ctrl+Shift+S) to keep the empty form as it was.

Each kind of field:

- **Text fields:** a click opens a text box over the field. Enter (in a one-line field), Esc, Tab or clicking
  elsewhere keeps what you typed, and so do saving, printing and other commands, so you can print or save straight
  from the field. While you type, Ctrl+Z and Ctrl+Y undo and redo the typing in that field only; once it is kept,
  Ctrl+Z takes it back. A value LimarePDF can't write (right-to-left text, say) stays in the field, with the
  reason shown, and the command doesn't run. Password fields show dots, and their value isn't kept in the file.
- **Checkboxes and radio buttons:** a click ticks or chooses. With the keyboard, Space ticks.
- **Drop-downs and lists:** a click opens the options (so do Space and Enter). In a list that allows several
  choices, each click adds or removes one at once; a click outside the list, Esc or Enter closes it. A drop-down
  you can type in has a text box at the top; Enter keeps the text.

Each value you change is one undo step. Hovering over a field shows its description.

## Highlighted fields

Fields are highlighted from the start: a light tint on every field, stronger under the pointer, and a red outline
on required ones. **Forms and sign > Fill fields ▾ > Highlight fields** (or ☰ > View > Highlight fields), or the
switch on the form's notice, turns this off or on again for every document, and the choice is remembered between
runs.

## The Form section

With nothing selected, the inspector's **Form** section shows the field you are in: its name, its description and
whether it is required. It also has these, which are in **Forms and sign > Fill fields ▾** too:

- **Clear field**, which empties the field you are in.
- **Highlight fields**, the same switch as above.
- **Reset form**, which puts every field back to its default.
- **Flatten form…** (also in ☰ > Document and the command palette), which makes the fields part of the pages as
  they look now, so nobody can change the values, here or in other readers. It asks first, and Undo brings the
  fields back until you close the document.

## Signature fields

- Clicking an empty signature field puts your fill-and-sign signature in it (see [Fill and sign](fill-and-sign.md)).
- Clicking a digitally signed one shows its signature's details (see [Digital signatures](signatures.md)).
- To sign a field with a digital ID, see [Signing](signatures.md#signing).

## What can't be filled in

- Read-only fields can't be changed, and buttons don't work (they run scripts, which LimarePDF doesn't). Hovering
  over one says why.
- Forms made only for Adobe Reader (XFA forms) can't be shown or filled in. A notice says so, and LimarePDF shows
  the placeholder page the file contains, often "Please wait...". Forms with both kinds of field are filled in
  through their ordinary fields.
- Form data files (FDF, XFDF) can't be imported or exported, and form JavaScript doesn't run, so fields that
  calculate or check values don't.
- Forms can't be combined: appending a file with form fields draws its fields onto its pages with their answers,
  after asking, and they can't be filled in afterwards (see
  [Add pages, and merge documents](organising-pages.md#add-pages-and-merge-documents)).

## How values are written

LimarePDF draws the filled fields itself, so the values show the same in other readers. Text must be Latin, Greek
or Cyrillic for now; anything else is refused with a message rather than saved wrongly, and stays typed in the
field, which gets the keyboard back once you close the message, so you can change it. A value the field's own
font can't show is drawn in the Liberation face of its kind, embedded in the file. Fields that have a value but no
drawing of their own are drawn when the file opens.

A value filled in by another app in other characters (Japanese, for example) shows only if that app drew it. If it
didn't, LimarePDF can't either: Flatten form… stops and names the fields rather than lose their values, and Print
asks whether to print without them, since they would come out empty on paper.

## From the command line

`limarepdf-cli form` lists a form's fields, fills them in (one by one, or from a JSON file), resets them and
flattens them: see [Forms](cli.md#forms).

---

[← Editing text and pictures](content-editing.md) · [User guide](README.md) · [Fill and sign →](fill-and-sign.md)
