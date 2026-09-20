# Compression fixture

`lz4-java.bin` was generated with the Java library `org.lz4:lz4-java:1.8.0`, using `LZ4BlockOutputStream` with a 65,536-byte block size and `LZ4Factory.fastestJavaInstance().fastCompressor()`. The uncompressed content is the ASCII string `chunklens` repeated 20,000 times (180,000 bytes).

This fixture exercises multiple compressed Java blocks, checksums, and the end marker independently of ChunkLens's test encoder. The generation uses the standard [Java LZ4 stream format](https://github.com/lz4/lz4-java/blob/master/src/java/net/jpountz/lz4/LZ4BlockOutputStream.java).
