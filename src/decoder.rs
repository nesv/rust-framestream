use crate::constants::{CONTROL_FIELD_CONTENT_TYPE, CONTROL_START, CONTROL_STOP};
use byteorder::{BigEndian, ReadBytesExt};
use std::{
    io::{Error, ErrorKind, Read, Result},
    iter::Iterator,
};

const MAX_CONTROL_FRAME_LENGTH: usize = 512;

#[derive(Clone, Debug)]
pub struct Decoder<R: Read> {
    reader: R,
    //bidirectional: Option<EncoderWriter>,
    content_type: Option<String>,
    started: bool,
}

impl<R: Read> Decoder<R> {
    /// Instantiate a new Decoder that can read from the given `source`.
    pub fn new(source: R) -> Self {
        Self {
            reader: source,
            // bidirectional: false,
            content_type: None,
            started: false,
        }
    }

    /// Limit the messages returned by the decoder to those with the specified
    /// content type `ctype`.
    pub fn content_type(&mut self, ctype: &str) {
        self.content_type = Some(ctype.to_owned());
    }

    /// Enable bidirectional mode for this decoder, by providing an
    /// `EncoderWriter`.
    // pub fn bidirectional(&mut self, enc: EncoderWriter) {
    //     self.bidirectional = Some(enc);
    // }

    fn read_control_frame(&mut self) -> Result<ControlFrame> {
        let frame_len = self.reader.read_u32::<BigEndian>()? as usize;
        if frame_len > MAX_CONTROL_FRAME_LENGTH {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("control frame too large: len={}", frame_len),
            ));
        } else if frame_len < 4 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "control frame too short",
            ));
        }

        // Read the frame's control type.
        let control_type = self.reader.read_u32::<BigEndian>()?;

        // Read the remainder of the buffer.
        let mut content_types = Vec::new();
        let mut remaining = frame_len - 4;
        while remaining > 8 {
            let (ctype, n) = self.read_control_field(remaining)?;
            content_types.push(ctype);
            remaining -= n;
        }

        Ok(ControlFrame {
            control_type,
            content_types: Some(content_types),
        })
    }

    /// Read a control field from `self.reader`, ensuring the field's size is
    /// less-than-or-equal-to `limit`.
    fn read_control_field(&mut self, limit: usize) -> Result<(String, usize)> {
        let field_type = self.reader.read_u32::<BigEndian>()?;
        if field_type != CONTROL_FIELD_CONTENT_TYPE {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("expected control field content type, got {:x}", field_type),
            ));
        }

        let field_len = self.reader.read_u32::<BigEndian>()? as usize;
        dbg!(field_len, limit);
        if field_len > limit {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "field contents too large (len={} limit={})",
                    field_len, limit
                ),
            ));
        }

        let mut buf = Vec::with_capacity(field_len);
        buf.resize(field_len, 0);
        self.reader.read_exact(&mut buf)?;

        let content_type = String::from_utf8_lossy(buf.as_slice()).into_owned();
        let bytes_read = 8 + field_len;

        Ok((content_type, bytes_read))
    }

    fn read_start_frame(&mut self) -> Result<()> {
        // Make sure the next four bytes are 0.
        let n = self.reader.read_u32::<BigEndian>()?;
        if n != 0 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "control start frame did not start with zero",
            ));
        }

        let frame = self.read_control_frame()?;
        if frame.control_type == CONTROL_START {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidInput,
                "expected control start frame",
            ))
        }
    }

    fn read_frame_length(&mut self) -> Result<usize> {
        let n = self.reader.read_u32::<BigEndian>()?;
        Ok(n as usize)
    }

    fn read_n(&mut self, n: usize, buf: &mut [u8]) -> Result<usize> {
        if n > buf.len() {
            Err(Error::new(
                ErrorKind::Other,
                "data frame too large for buffer",
            ))
        } else {
            match self.reader.read_exact(&mut buf[..n]) {
                Ok(_) => Ok(n),
                Err(e) => Err(e),
            }
        }
    }
}

struct ControlFrame {
    control_type: u32,

    #[allow(dead_code)]
    content_types: Option<Vec<String>>,
}

impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // If we have not read the CONTROL_START frame yet, read it now.
        if !self.started {
            self.read_start_frame()?;
            self.started = true;
        }

        // Read the frame length.
        let frame_len = self.read_frame_length()?;
        if frame_len == 0 {
            // This is a control frame.
            let frame = self.read_control_frame()?;
            if frame.control_type == CONTROL_STOP {
                // if let Some(ref mut encoder) = self.bidirectional {
                //     // TODO: Write a CONTROL_FINISH frame.
                // }
            }
            return Ok(0);
        }

        // Read the data into the buffer.
        self.read_n(frame_len, &mut buf[..])
    }
}

#[derive(Debug, PartialEq)]
pub struct Frame {
    data: Vec<u8>,
}

impl<R: Read> Iterator for Decoder<R> {
    type Item = Frame;
    fn next(&mut self) -> Option<Self::Item> {
        if !self.started {
            self.read_start_frame().ok()?;
            self.started = true;
        }

        let frame_len = self.read_frame_length().ok()?;
        if frame_len == 0 {
            // Control frame.
            let frame = self.read_control_frame().ok()?;
            if frame.control_type == CONTROL_STOP {
                // TODO(nesv): Write a CONTROL_FINISH frame.
                return None;
            }
        }

        let mut buf = Vec::with_capacity(frame_len);
        buf.resize(frame_len, 0);
        match self.read_n(frame_len, &mut buf[..]) {
            Ok(_) => Some(Frame { data: buf }),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
#[test]
fn iter() {
    let input = std::io::Cursor::new([
        0, 0, 0, 0, 0, 0, 0, 29, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 17, 116, 101, 115, 116, 45, 99,
        111, 110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 0, 0, 0, 12, 116, 101, 115, 116, 45,
        99, 111, 110, 116, 101, 110, 116, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 3,
    ]);
    let mut decoder = Decoder::new(input);
    let want = "test-content".as_bytes().to_vec();
    assert_eq!(decoder.next(), Some(Frame { data: want }));
    assert_eq!(decoder.next(), None);
}

#[test]
fn read() {
    let input = std::io::Cursor::new([
        0, 0, 0, 0, 0, 0, 0, 29, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 17, 116, 101, 115, 116, 45, 99,
        111, 110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 0, 0, 0, 12, 116, 101, 115, 116, 45,
        99, 111, 110, 116, 101, 110, 116, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 3,
    ]);
    let mut decoder = Decoder::new(input);
    let mut buf = [0; 1 << 10];
    let n = decoder.read(&mut buf[..]).unwrap();
    let got = std::str::from_utf8(&buf[..n]).unwrap();
    assert_eq!(got, "test-content");
}

#[test]
fn read_start_frame() {
    let input = std::io::Cursor::new([
        0, 0, 0, 0, 0, 0, 0, 29, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 17, 116, 101, 115, 116, 45, 99,
        111, 110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 0, 0, 0, 12, 116, 101, 115, 116, 45,
        99, 111, 110, 116, 101, 110, 116, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 3,
    ]);
    let mut decoder = Decoder::new(input);
    decoder.read_start_frame().unwrap();
}

#[test]
fn read_control_frame() {
    let input = std::io::Cursor::new([
        0, 0, 0, 29, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 17, 116, 101, 115, 116, 45, 99, 111, 110,
        116, 101, 110, 116, 45, 116, 121, 112, 101,
    ]);
    let mut decoder = Decoder::new(input);
    let control_frame = decoder.read_control_frame().unwrap();
    assert_eq!(
        control_frame.content_types,
        Some(vec!["test-content-type".to_string()])
    );
}

#[test]
fn read_control_field() {
    let input = std::io::Cursor::new([
        0, 0, 0, 1, 0, 0, 0, 17, 116, 101, 115, 116, 45, 99, 111, 110, 116, 101, 110, 116, 45, 116,
        121, 112, 101,
    ]);
    let mut decoder = Decoder::new(input);
    let (ctype, bytes_read) = decoder.read_control_field(29).unwrap();
    assert_eq!(&ctype, "test-content-type");
    assert_eq!(bytes_read, 25);
}
