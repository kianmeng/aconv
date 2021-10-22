
use encoding_rs as enc;
use crate::Transcoder;

pub struct TranscodingReader<R: std::io::Read> {
    reader: R,
    bytes_to_guess: usize,
    non_ascii_to_guess: usize,
    non_text_threshold: u8,
    buffer: Vec<u8>,
    unread_buffer: Vec<u8>,
    unwritten_buffer: Vec<u8>,
    transcoder: Transcoder,
    had_replacement_or_unmappable: bool,
    transcode_done: bool,
    eof: bool,
    no_transcoding_needed: bool,
}

impl <R: std::io::Read> TranscodingReader<R> {
    pub fn new(reader: R, src_encoding: Option<&'static enc::Encoding>, dst_encoding: &'static enc::Encoding) -> Self {
        return TranscodingReader {
            reader,
            bytes_to_guess: 1024,
            non_ascii_to_guess: 100,
            non_text_threshold: 0,
            buffer: vec![0u8; 8*1024],
            unread_buffer: vec![],
            unwritten_buffer: vec![],
            transcoder: Transcoder::new(src_encoding, dst_encoding),
            had_replacement_or_unmappable: false,
            transcode_done: false,
            eof: false,
            no_transcoding_needed: false,
        };
    }

    pub fn buffer_size(mut self: Self, size: usize) -> Self {
        if size < 16 { // if the buffer is insufficient, let's ignore the specified size.
            return self;
        }
        self.buffer = vec![0u8; size];
        return self;
    }

    pub fn bytes_to_guess(mut self: Self, size: usize) -> Self {
        self.bytes_to_guess = size;
        return self;
    }

    pub fn non_text_threshold(mut self: Self, percent: u8) -> Self {
        self.non_text_threshold = percent;
        return self;
    }

    pub fn non_ascii_to_guess(mut self: Self, num: usize) -> Self {
        self.non_ascii_to_guess = num;
        return self;
    }

    pub fn guess(self: &mut Self)
        -> std::io::Result<(Option<&'static enc::Encoding>, bool)> {
        let read_buf = &mut vec![0u8; self.bytes_to_guess];
        let buf_minus1 =read_buf.len()-1;
        let first = self.reader.read(&mut read_buf[..buf_minus1])?;
        let is_empty = first == 0;
        if is_empty {
            self.eof = true;
            self.transcode_done = true;
            return Ok((None, is_empty));
        }
        let second = self.reader.read(&mut read_buf[buf_minus1..])?;
        self.eof = second == 0;
        let n = first +second;
        let src = &read_buf[..n];
        // TODO new transcoder
        let rslt = self.transcoder.guess_and_transcode(src, &mut self.buffer, self.non_ascii_to_guess, self.non_text_threshold, self.eof);
        let (guessed_enc_opt, coder_result, num_read, num_written, has_replacement) = rslt;
        self.no_transcoding_needed = guessed_enc_opt.is_none()
            || (guessed_enc_opt.is_some() && guessed_enc_opt.unwrap() == self.transcoder.dst_encoding);
        if self.no_transcoding_needed {
            self.unwritten_buffer = src.to_owned();
            return Ok((guessed_enc_opt, is_empty));
        } else {
            self.transcode_done = (coder_result == enc::CoderResult::InputEmpty) && self.eof;
            self.had_replacement_or_unmappable = has_replacement;
            self.unread_buffer = src[num_read..].into();
            self.unwritten_buffer = {
                if self.transcoder.dst_encoding() == enc::UTF_16BE && [0xFE,0xFF] != self.buffer[..2] {
                    [b"\xFE\xFF", &self.buffer[..num_written]].concat() // add a BOM
                } else if self.transcoder.dst_encoding() == enc::UTF_16LE && [0xFF,0xFE] != self.buffer[..2] {
                    [b"\xFF\xFE", &self.buffer[..num_written]].concat() // add a BOM
                } else{
                    self.buffer[..num_written].into()
                }
            };
            return Ok((guessed_enc_opt, is_empty));
        }
    }

    fn copy_from_unwritten_buffer_to(self: &mut Self, buffer: &mut [u8]) -> usize{
        let min = std::cmp::min(buffer.len(), self.unwritten_buffer.len());
        buffer[..min].copy_from_slice(&self.unwritten_buffer[..min]);
        self.unwritten_buffer = self.unwritten_buffer[min..].into();
        return min;
    }

    fn run_transcode(self: &mut Self, buffer: &mut[u8]) -> usize {
        let src = &mut self.unread_buffer;

        if src.len() == 0 && !self.eof { // encoding_rs unable to handle unnecessary calls well, so let's skip them
            return 0;
        }

        if buffer.len() > 16 { // buffer has enough bytes for encoding_rs to write output
            let rslt = self.transcoder.transcode(src, buffer, self.eof);
            let (coder_result, num_read, num_written, has_replacement) = rslt;
            self.unread_buffer = src[num_read..].into();
            self.had_replacement_or_unmappable = self.had_replacement_or_unmappable || has_replacement;
            self.transcode_done = (coder_result == enc::CoderResult::InputEmpty) && self.eof;
            if num_written > 0 {
                return num_written;
            }
        } else { // if the buffer is insufficient, let's create a buffer by ourselves
            let write_buffer = &mut [0u8; 8*1024];
            let rslt = self.transcoder.transcode(src, write_buffer, self.eof);
            let (coder_result, num_read, num_written, has_replacement) = rslt;
            self.unread_buffer = src[num_read..].into();
            self.unwritten_buffer = write_buffer[..num_written].into();
            self.had_replacement_or_unmappable = self.had_replacement_or_unmappable || has_replacement;
            self.transcode_done = (coder_result == enc::CoderResult::InputEmpty) && self.eof;
            if self.unwritten_buffer.len() > 0 {
                let n = self.copy_from_unwritten_buffer_to(buffer);
                return n;
            }
        }

        return 0;
    }
}

impl <R: std::io::Read> std::io::Read for TranscodingReader<R> {

    fn read(self: &mut Self, buffer: &mut [u8]) -> std::io::Result<usize> {

        if buffer.len() == 0 {
            return Ok(0);
        }

        if self.unwritten_buffer.len() > 0 {
            let num_written = self.copy_from_unwritten_buffer_to(buffer);
            return Ok(num_written);
        }

        if self.no_transcoding_needed {
            let n = self.reader.read(buffer)?;
            return Ok(n);
        }

        if self.transcode_done {
            return Ok(0);
        }

        if self.unread_buffer.len() > 0 {
            let num_written = self.run_transcode(buffer);
            if num_written > 0 {
                return Ok(num_written);
            }
        }

        let n = self.reader.read(&mut self.buffer)?;
        self.unread_buffer = self.buffer[..n].into();
        self.eof = n == 0;
        let num_written = self.run_transcode(buffer);
        return Ok(num_written);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::path;
    use super::*;

    // macro_rules! test_reader {
    //     ($name:ident, $input_file:expr, $expected_file:expr, $enc:expr, $read_buff_size:expr) => {
    //         #[test]
    //         fn $name() {
    //             let test_data = path::Path::new("../test_data");
    //             let ifile_handle = &mut std::fs::File::open(test_data.join($input_file)).unwrap();
    //             let enc = enc::Encoding::for_label($enc.as_bytes());
    //             let t = &mut TranscodingReader::new(ifile_handle, None, enc.unwrap())
    //                     .buffer_size(128)
    //                     .bytes_to_guess(256);
    //             let mut buff = vec![0u8; $read_buff_size];
    //             t.guess().unwrap();
    //             let n = t.read_to_end(&mut buff).unwrap();
    //             let efile_handle = &mut std::fs::File::open(test_data.join($expected_file)).unwrap();
    //             let expected_string = &mut Vec::new();
    //             efile_handle.read_to_end(expected_string).unwrap();
    //             assert_eq!(&expected_string[..n], &buff[..n]);
    //         }
    //     };
    // }

    macro_rules! test_reader {
        ($name:ident, $input_file:expr, $expected_file:expr, $enc:expr) => {
            #[test]
            fn $name() {
                let test_data = path::Path::new("../test_data");
                let ifile_handle = &mut std::fs::File::open(test_data.join($input_file)).unwrap();
                let enc = enc::Encoding::for_label($enc.as_bytes());
                let t = &mut TranscodingReader::new(ifile_handle, None, enc.unwrap());
                let mut buff = Vec::new();
                t.guess().unwrap();
                t.read_to_end(&mut buff).unwrap();
                let efile_handle = &mut std::fs::File::open(test_data.join($expected_file)).unwrap();
                let mut expected_string = Vec::new();
                efile_handle.read_to_end(&mut expected_string).unwrap();
                // assert!(expected_string == buff);
            }
        };
    }

    test_reader!(reader_sjis_utf8        , "sjis_ja.txt"         , "utf8_ja.txt"     , "utf8");

    test_reader!(reader_utf8_euckr       , "utf8_ko.txt"     , "euc-kr_ko.txt"       , "euc-kr");

//    test_guess!(guess_utf16le_utf8     , "utf16le_BOM_th.txt"  , "utf8_th.txt"     , "utf8");
//    test_guess!(guess_utf16be_utf8     , "utf16be_BOM_th.txt"  , "utf8_th.txt"     , "utf8");
//    test_guess!(guess_sjis_utf8        , "sjis_ja.txt"         , "utf8_ja.txt"     , "utf8");
//    test_guess!(guess_eucjp_utf8       , "euc-jp_ja.txt"       , "utf8_ja.txt"     , "utf8");
//    test_guess!(guess_iso2022jp_utf8   , "iso-2022-jp_ja.txt"  , "utf8_ja.txt"     , "utf8");
//    test_guess!(guess_big5_utf8        , "big5_zh_CHT.txt"     , "utf8_zh_CHT.txt" , "utf8");
//    test_guess!(guess_gbk_utf8         , "gbk_zh_CHS.txt"      , "utf8_zh_CHS.txt" , "utf8");
//    test_guess!(guess_gb18030_utf8     , "gb18030_zh_CHS.txt"  , "utf8_zh_CHS.txt" , "utf8");
//    test_guess!(guess_euckr_utf8       , "euc-kr_ko.txt"       , "utf8_ko.txt"     , "utf8");
//    test_guess!(guess_koi8r_utf8       , "koi8-r_ru.txt"       , "utf8_ru.txt"     , "utf8");
//    test_guess!(guess_windows1252_utf8 , "windows-1252_es.txt" , "utf8_es.txt"     , "utf8");
//
//    test_guess!(guess_utf8_utf16le     , "utf8_th.txt"     , "utf16le_th.txt"      , "utf-16le"     );
//    test_guess!(guess_utf8_utf16be     , "utf8_th.txt"     , "utf16be_th.txt"      , "utf-16be"     );
//    test_guess!(guess_utf8_sjis        , "utf8_ja.txt"     , "sjis_ja.txt"         , "sjis"         );
//    test_guess!(guess_utf8_eucjp       , "utf8_ja.txt"     , "euc-jp_ja.txt"       , "euc-jp"       );
//    test_guess!(guess_utf8_iso2022jp   , "utf8_ja.txt"     , "iso-2022-jp_ja.txt"  , "iso-2022-jp"  );
//    test_guess!(guess_utf8_big5        , "utf8_zh_CHT.txt" , "big5_zh_CHT.txt"     , "big5"         );
//    test_guess!(guess_utf8_gbk         , "utf8_zh_CHS.txt" , "gbk_zh_CHS.txt"      , "gbk"          );
//    test_guess!(guess_utf8_gb18030     , "utf8_zh_CHS.txt" , "gb18030_zh_CHS.txt"  , "gb18030"      );
//    test_guess!(guess_utf8_euckr       , "utf8_ko.txt"     , "euc-kr_ko.txt"       , "euc-kr"       );
//    test_guess!(guess_utf8_koi8r       , "utf8_ru.txt"     , "koi8-r_ru.txt"       , "koi8-r"       );
//    test_guess!(guess_utf8_windows1252 , "utf8_es.txt"     , "windows-1252_es.txt" , "windows-1252" );
}
