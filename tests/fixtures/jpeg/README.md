Synthetic JPEG fixtures generated for NoirView with Pillow, without external images:

- progressive.jpg: 800x600 RGB gradient, quality 95, progressive encoding.
- gray.jpg: luminance conversion of that gradient, quality 95.
- cmyk.jpg: 160x120 CMYK gradient with independently varying C/M/Y and K, quality 95.

They exercise reduced decoding, color conversion and coefficient-memory checks. They are not photo-throughput benchmarks. Orientation fixtures are generated independently by Rust tests for all eight EXIF values and both TIFF byte orders.
