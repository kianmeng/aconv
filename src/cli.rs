use crate::option;
use crate::transcode;
use crate::error;

use encoding_rs as enc;
use transcoding_rs::constants;
use std::io;
use std::fs;
use std::path;

pub fn dispatch(opt: &option::Opt) -> Result<(), error::Error> {
    if opt.list {
        list();
        return Ok(());
    } else {
        return run(opt);
    }
}

fn run(opt: &option::Opt) -> Result<(), error::Error> {
    let input_buffer = &mut [0u8; 5*1024]; // 5K bytes
    let output_buffer = &mut [0u8; 10*1024]; // 10K bytes

    let to_code = match enc::Encoding::for_label(opt.to_code.as_bytes()) {
        None => return Err(error::Error::other(&format!("Invalid encoding: {}", opt.to_code))),
        Some(e) => e,
    };

    let stdout = std::io::stdout();
    let mut stdout_lock;
    let mut writer: Option<&mut dyn io::Write>=None;
    let mut dir_opt: Option<&path::PathBuf>=None;
    let in_paths = &opt.paths;
    match opt.output.as_ref() {
        Some(out_path) => {
            if in_paths.len() == 0 {
                stdout_lock = stdout.lock();
                writer = Some(&mut stdout_lock);
            } else {
                dir_opt = Some(out_path);
            }
        },
        None => {
            stdout_lock = stdout.lock();
            writer = Some(&mut stdout_lock);
        },
    };

    if in_paths.len() == 0 {
        let stdin = &mut std::io::stdin();
        let rslt = transcode::transcode(stdin, writer.unwrap(), to_code, input_buffer, output_buffer, &opt);
        match rslt {
            Ok(enc) => {
                if opt.show {
                    println!("-: {}", enc.name());
                }
                return Ok(());
            },
            Err(err) => {
                if let error::TranscodeError::Guess(msg) = &err {
                    if ! opt.quiet {
                        eprintln!("{}", msg);
                    }
                }
                return Err(err.into());
            }
        }
    } else {
        for i in 0..in_paths.len() {
            let in_path = &in_paths[i];
            if ! path::Path::exists(&in_paths[i]) {
                let source = io::Error::new(io::ErrorKind::NotFound, "No such file or directory");
                return Err(error::Error::Io { source, path: in_path.to_owned(), message: "Error opening the file".into() });
            }
        }
        for i in 0..in_paths.len() {
            let in_path = &fs::canonicalize(&in_paths[i])
                .map_err(|e| error::Error::Io { source: e, path: in_paths[i].to_owned(), message: "Error reading the file or directory".into() })?;
            traverse(&mut writer, to_code, input_buffer, output_buffer, in_path, dir_opt ,opt)?;
        }
        return Ok(());
    }
}

fn traverse(writer_opt: &mut Option<&mut dyn io::Write>, to_code: &'static enc::Encoding, input_buffer: &mut [u8], output_buffer: &mut [u8]
    , in_path: &path::PathBuf, dir_opt: Option<&path::PathBuf>, opt: &option::Opt)
    -> Result<(), error::Error> {
    if in_path.is_dir() {
        let dir_ent = fs::read_dir(in_path)
            .map_err(|e| error::Error::Io { source: e, path: in_path.to_owned(), message: "Error reading the directory".into() })?;
        for child in dir_ent {
            let c = child
                .map_err(|e| error::Error::Io { source: e, path: in_path.to_owned(), message: "Error reading the directory".into() })?;
            let child_path = &c.path();
            if let Some(current_out_dir) = dir_opt {
                let out_dir = &current_out_dir.join(in_path.file_name().unwrap());
                traverse(&mut None, to_code, input_buffer, output_buffer, child_path, Some(out_dir), opt)?;
            } else {
                traverse(writer_opt, to_code, input_buffer, output_buffer, child_path, None, opt)?;
            }
        }
        return Ok(());
    } else {
        let mut ofile;
        let writer: &mut dyn io::Write = if let Some(dir_path) = dir_opt {
            create_dir_recursive(dir_path)?;
            let out_path = &dir_path.join(in_path.file_name().unwrap());
            ofile =fs::File::create(out_path)
                .map_err(|e| error::Error::Io { source: e, path: out_path.to_owned(), message: "Error creating the file".into() })?;
            println!("ofile: {:?}", ofile);
            &mut ofile
        } else {
            writer_opt.as_mut().unwrap()
        };
        let reader = &mut fs::File::open(in_path)
            .map_err(|e| error::Error::Io { source: e, path: in_path.to_owned(), message: "Error opening the file".into() })?;
        println!("output buffer length: {}", output_buffer.len());
        let rslt = transcode::transcode(reader, writer, to_code, input_buffer, output_buffer, &opt);
        match rslt {
            Ok(enc) => {
                if opt.show {
                    println!("{}: {}", in_path.to_string_lossy(), enc.name());
                }
                return Ok(());
            },
            Err(err) => {
                if let error::TranscodeError::Read(source) = err {
                    return Err(error::Error::Io { source, path: in_path.to_owned(), message: "Error reading the file".into() });
                }
                if let error::TranscodeError::Write(source) = err {
                    return Err(error::Error::Io { source, path: in_path.to_owned(), message: "Error writing the file".into() });
                }
                if let error::TranscodeError::Guess(msg) = &err {
                    if ! opt.quiet {
                        eprintln!("{}", msg);
                    }
                }
                return Err(err.into());
            }
        }
    }
}

fn create_dir_recursive(path: &path::PathBuf)
    -> Result<(), error::Error> {
    let path_to_create = &mut path::PathBuf::new();
    for p in path.iter() {
        path_to_create.push(p);
        if ! path_to_create.is_dir() {
            fs::create_dir(&path_to_create)
                .map_err(|e| error::Error::Io { source: e, path: path_to_create.to_owned(), message: "Error creating the directory".into() })?;
        }
    }
    return Ok(());
}

fn list() {
    print!("{}", constants::ENCODINGS[0].1);
    for i in 1..constants::ENCODINGS.len() {
        let encoding = constants::ENCODINGS[i];
        if constants::ENCODINGS[i-1].0 == encoding.0 {
            print!(" ");
        } else {
            println!();
        }
        print!("{}", encoding.1);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_to_dir() {
        let opt = &mut option::Opt::new()
            .paths(vec![path::PathBuf::from("test_data/child")])
            .output(Some(path::PathBuf::from("output")));
        run(opt).unwrap();
    }

    #[test]
    fn files_to_dir() {
        let opt = &mut option::Opt::new()
            .paths(vec![path::PathBuf::from("test_data/child")])
            .output(Some(path::PathBuf::from("output")));
        run(opt).unwrap();
    }

    #[test]
    fn to_sjis() {
        let opt = &mut option::Opt::new()
            .paths(vec![path::PathBuf::from("test_data/utf8_ja.txt")])
            .to_code("sjis");
        run(opt).unwrap();
    }

    #[test]
    fn to_list() {
        let opt = &mut option::Opt::new()
            .list(true);
        run(opt).unwrap();
    }

    #[test]
    fn to_show() {
        let opt = &mut option::Opt::new()
            .show(true);
        run(opt).unwrap();
    }
}

