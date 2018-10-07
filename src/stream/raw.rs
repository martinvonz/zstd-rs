//! Raw in-memory stream compression/decompression.
//!
//! This module defines a `Decoder` and an `Encoder` to decode/encode streams
//! of data using buffers.
//!
//! They are mostly thin wrappers around `zstd_safe::{DStream, CStream}`.
use std::io;

use zstd_safe::{self, CStream, DStream, InBuffer, OutBuffer};

use parse_code;

/// Represents an abstract compression/decompression operation.
///
/// This trait covers both `Encoder` and `Decoder`.
pub trait Operation {
    /// Performs a single step of this operation.
    ///
    /// Should return a hint for the next input size.
    ///
    /// If the result is `Ok(0)`, it may indicate that a frame was just
    /// finished.
    fn run(
        &mut self,
        input: &mut InBuffer,
        output: &mut OutBuffer,
    ) -> io::Result<usize>;

    /// Performs a single step of this operation.
    ///
    /// This is a comvenience wrapper around `Operation::run` if you don't
    /// want to deal with `InBuffer`/`OutBuffer`.
    fn run_on_buffers(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> io::Result<Status> {
        let mut input = InBuffer::around(input);
        let mut output = OutBuffer::around(output);

        let remaining = self.run(&mut input, &mut output)?;

        Ok(Status {
            remaining,
            bytes_read: input.pos,
            bytes_written: output.pos,
        })
    }

    /// Prepares this operation for a new frame.
    ///
    /// If `Self::run()` returns `Ok(0)`, and more data needs to be processed,
    /// this will be called.
    ///
    /// Mostly used for decoder to handle concatenated frames.
    fn reinit(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Flushes any internal buffer, if any.
    ///
    /// Returns the number of bytes still in the buffer.
    fn flush(&mut self, output: &mut OutBuffer) -> io::Result<usize> {
        Ok(0)
    }

    /// Finishes the operation, writing any footer if necessary.
    ///
    /// Returns the number of bytes still to write.
    ///
    /// Keep calling this method until it returns `Ok(0)`,
    /// and then don't ever call this method.
    fn finish(&mut self, output: &mut OutBuffer) -> io::Result<usize> {
        Ok(0)
    }
}

/// Dummy operation that just copies its input to the output.
pub struct NoOp;

impl Operation for NoOp {
    fn run(
        &mut self,
        input: &mut InBuffer,
        output: &mut OutBuffer,
    ) -> io::Result<usize> {
        let src = &input.src[input.pos..];
        let dst = &mut output.dst[output.pos..];

        let len = usize::min(src.len(), dst.len());
        let src = &src[..len];
        let dst = &mut dst[..len];

        dst.copy_from_slice(src);
        input.pos += len;
        output.pos += len;

        Ok(0)
    }
}

/// Describes the result of an operation.
pub struct Status {
    /// Number of bytes expected for next input.
    ///
    /// This is just a hint.
    pub remaining: usize,

    /// Number of bytes read from the input.
    pub bytes_read: usize,

    /// Number of bytes written to the output.
    pub bytes_written: usize,
}

/// An in-memory decoder for streams of data.
pub struct Decoder {
    context: DStream,
}

/// An in-memory encoder for streams of data.
pub struct Encoder {
    context: CStream,
}

impl Decoder {
    /// Creates a new decoder.
    pub fn new() -> io::Result<Self> {
        Self::with_dictionary(&[])
    }

    /// Creates a new decoder initialized with the given dictionary.
    pub fn with_dictionary(dictionary: &[u8]) -> io::Result<Self> {
        let mut context = zstd_safe::create_dstream();
        parse_code(zstd_safe::init_dstream_using_dict(
            &mut context,
            dictionary,
        ))?;
        Ok(Decoder { context })
    }
}

impl Operation for Decoder {
    fn run(
        &mut self,
        input: &mut InBuffer,
        output: &mut OutBuffer,
    ) -> io::Result<usize> {
        parse_code(zstd_safe::decompress_stream(
            &mut self.context,
            output,
            input,
        ))
    }

    fn reinit(&mut self) -> io::Result<()> {
        parse_code(zstd_safe::reset_dstream(&mut self.context))?;
        Ok(())
    }
}

impl Encoder {
    /// Creates a new encoder.
    pub fn new(level: i32) -> io::Result<Self> {
        Self::with_dictionary(level, &[])
    }

    /// Creates a new encoder initialized with the given dictionary.
    pub fn with_dictionary(level: i32, dictionary: &[u8]) -> io::Result<Self> {
        let mut context = zstd_safe::create_cstream();
        parse_code(zstd_safe::init_cstream_using_dict(
            &mut context,
            dictionary,
            level,
        ))?;
        Ok(Encoder { context })
    }
}

impl Operation for Encoder {
    fn run(
        &mut self,
        input: &mut InBuffer,
        output: &mut OutBuffer,
    ) -> io::Result<usize> {
        parse_code(zstd_safe::compress_stream(
            &mut self.context,
            output,
            input,
        ))
    }

    fn flush(&mut self, output: &mut OutBuffer) -> io::Result<usize> {
        parse_code(zstd_safe::flush_stream(&mut self.context, output))
    }

    fn finish(&mut self, output: &mut OutBuffer) -> io::Result<usize> {
        parse_code(zstd_safe::end_stream(&mut self.context, output))
    }
}

#[cfg(test)]
mod tests {
    use super::{Decoder, Encoder, Operation};
    use zstd_safe::{InBuffer, OutBuffer};

    #[test]
    fn test_cycle() {
        let mut encoder = Encoder::new(1).unwrap();
        let mut decoder = Decoder::new().unwrap();

        // Step 1: compress
        let mut input = InBuffer::around(b"AbcdefAbcdefabcdef");

        let mut output = [0u8; 128];
        let mut output = OutBuffer::around(&mut output);

        loop {
            encoder.run(&mut input, &mut output).unwrap();

            if input.pos == input.src.len() {
                break;
            }
        }
        encoder.finish(&mut output).unwrap();

        let initial_data = input.src;

        // Step 2: decompress

        let mut input = InBuffer::around(output.as_slice());
        let mut output = [0u8; 128];
        let mut output = OutBuffer::around(&mut output);

        loop {
            decoder.run(&mut input, &mut output).unwrap();

            if input.pos == input.src.len() {
                break;
            }
        }

        assert_eq!(initial_data, output.as_slice());
    }
}
