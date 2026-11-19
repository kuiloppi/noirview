# Independent IPTC interoperability fixture

`iptc-keywords.jpg` is a generated 31×23 RGB gradient, encoded with Pillow 11.1.0 (quality 91), then tagged by ExifTool 13.25, independently of NoirView's writer. It contains no personal photo or user data.

Pillow pixels: `((x*7)%256, (y*9)%256, 67)` for each `(x,y)`.

ExifTool generation arguments:

```text
-overwrite_original -charset IPTC=UTF8 -IPTC:CodedCharacterSet=UTF8
-IPTC:Keywords=järvi -IPTC:Keywords+=日本語 -IPTC:Keywords+=😀
-IPTC:Caption-Abstract=Säilytä kuvateksti -IPTC:By-line=Test Photographer
-IPTC:CopyrightNotice=IPTC copyright -EXIF:Artist=Independent Artist
-EXIF:Orientation#=6 -EXIF:XPKeywords=XP only -XMP-dc:Subject=XMP only
-Photoshop:IPTCDigest=new
-XMP-photoshop:LegacyIPTCDigest=00000000000000000000000000000000
```

The intentionally stale XMP reconciliation digest must be refreshed on write. Original Photoshop IPTCDigest: `f1c1b0092d67bfd0c92e4aab93f107c4`.

The default Rust tests read this fixture and exercise add, replace, one-word removal, clear, independent embedded field reads, sidecar authority, scan-byte preservation, malformed/ambiguous IPTC and failure-before-publication. Optional independent verification uses ExifTool `-G1 -s -IPTC:all -XPKeywords -Subject -IPTCDigest -LegacyIPTCDigest -Artist -Orientation` before/after, and Pillow compares decoded RGB bytes.

ExifTool 13.25's XPKeywords text conversion does not correctly combine emoji surrogate pairs. For that value, independently read Pillow `Image.open(path).getexif()[40094]` as bytes and decode strictly with `utf-16-le`, trimming only the terminal NUL. This verified that the written XPKeywords retains the same emoji as IPTC/XMP.
