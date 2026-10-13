# Word document preview fixture

`report.docx` is a hand-authored minimal OOXML package (no Word or LibreOffice
involved) holding one of each structure the DOCX preview maps:

- `Heading1` and `Heading2` styled paragraphs, and a `Quote` styled paragraph;
- a paragraph mixing bold, italic, underlined, and struck runs with `&` and `<`
  (underline carries `w:val="single"`, as Word writes it: `docx-rs` panics on a bare
  `<w:u/>`);
- a bulleted list with a nested second level (`numId` 1, `bullet` format);
- a numbered list (`numId` 2, `decimal` format);
- a three-row table whose first row supplies the column titles;
- a run containing a `<w:br/>` line break;
- an empty `<w:drawing>` run, which the preview must drop.

The package contains only `[Content_Types].xml`, `_rels/.rels`,
`word/_rels/document.xml.rels`, `word/document.xml`, `word/numbering.xml`, and
`word/styles.xml` — the parts `docx-rs` requires plus numbering and styles.

# PDF text-layer fixture

`text.pdf` is a hand-written minimal PDF (single Helvetica text stream, no
compression) whose two text lines let the quick preview exercise its text
layer: glyph hit-testing, drag selection, and copy.

# Password-protected PDF fixture

`locked.pdf` is `text.pdf` encrypted with qpdf 12.4.1 (AES-256, R6):

```bash
qpdf --encrypt strata-pdf-fixture strata-pdf-fixture 256 \
  -- tests/fixtures/documents/text.pdf tests/fixtures/documents/locked.pdf
```

Both the user and owner passwords are `strata-pdf-fixture`; use the wrong
password `not-the-fixture-password` for negative tests. Regenerate with the
same command if the fixture ever needs rebuilding.
