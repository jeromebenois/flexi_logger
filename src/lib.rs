#![doc(html_logo_url = "http://www.rust-lang.org/logos/rust-logo-128x128-blk-v2.png",
       html_favicon_url = "http://www.rust-lang.org/favicon.ico",
       html_root_url = "http://doc.rust-lang.org/")]

//! An extended copy of [env_logger](http://rust-lang.github.io/log/env_logger/), which
//! can write the log to standard error or to a fresh file in a configurable folder
//! and allows custom logline formats.
//!
//! # Usage
//!
//! This crate is on [crates.io](https://crates.io/crates/flexi_logger) and
//! can be used by adding `flexi_logger` to the dependencies in your
//! project's `Cargo.toml`.
//!
//! ```toml
//! [dependencies]
//! flexi_logger = "0.2"
//! ```
//!
//! and this to your crate root:
//!
//! ```rust
//! extern crate flexi_logger;
//! ```
//!
//! flexi_logger plugs into the logging facade given by the
//! [log crate](http://rust-lang.github.io/log/log/).
//! i.e., you use the ```log``` macros to write log lines from your code.
//!
//! In its initialization (see function [init](fn.init.html)), you can
//!
//! *  decide whether you want to write your logs to stderr (like with env_logger),
//!    or to a file,
//! *  configure the folder in which the log files are created,
//! *  provide the log-level-specification, i.e., the decision which log
//!    lines really should be written out, programmatically (if you don't want to
//!    use the environment variable RUST_LOG)
//! *  specify the line format for the log lines <br>
//!    (flexi_logger comes with two predefined variants for the log line format,
//!    ```default_format()``` and ```detailed_format()```,
//!    but you can easily create and use your own format function with the
//!    signature ```fn(&LogRecord) -> String```)

extern crate glob;
extern crate log;
extern crate regex;
extern crate time;

use glob::glob;
use log::{Log, LogLevel, LogLevelFilter, LogMetadata};
pub use log::LogRecord;
use regex::Regex;
use std::cell::RefCell;
use std::cmp::max;
use std::env;
use std::fmt;
use std::fs::{create_dir_all,File};
use std::io;
use std::io::{stderr, LineWriter, Write};
use std::ops::{Add,DerefMut};
use std::path::Path;
use std::sync::{Arc, Mutex};

macro_rules! print_err {
    ($($arg:tt)*) => (
        {
            use std::io::prelude::*;
            if let Err(e) = write!(&mut ::std::io::stderr(), "{}\n", format_args!($($arg)*)) {
                panic!("Failed to write to stderr.\
                    \nOriginal error output: {}\
                    \nSecondary error writing to stderr: {}", format!($($arg)*), e);
            }
        }
    )
}

// Encapsulation for LineWriter-related stuff
struct OptLineWriter {
    olw: Option<LineWriter<File>>,
    o_filename_base: Option<String>,
    use_rolling: bool,
    written_bytes: usize,
    roll_idx: usize,
}
impl OptLineWriter {
    fn new (config: &LogConfig) -> OptLineWriter {
        if !config.log_to_file {
            // we don't need a line-writer, so we return a meaningless "empty" instance
            return OptLineWriter{olw: None, o_filename_base: None, use_rolling: false, written_bytes: 0, roll_idx: 0};
        }

        // make sure the folder exists or can be created
        let s_directory: String = match config.directory {
            Some(ref dir) => dir.clone(),
            None => ".".to_string()
        };
        let directory = Path::new(&s_directory);

        create_dir_all(&directory).unwrap_or(
            print_err!("Log cannot be written: output directory \"{}\" does not exist and could not be created",
                       &directory.display()));

        let o_filename_base = match std::fs::metadata(&directory) {
            Ok(metadata) => {
                if metadata.is_dir() {
                    Some(get_filename_base(&s_directory.clone(), & config.o_discriminant))
                } else {
                    None
                }
            },
            Err(_) => None
        };

        let (use_rolling, roll_idx) = match o_filename_base {
            None => (false, 0),
            Some(ref s_filename_base) => {
                match config.rollover_size {
                    None => (false, 0),
                    Some(_) => (true, get_next_roll_idx(&s_filename_base, & config.suffix))
                }
            }
        };

        let mut olw = OptLineWriter{
            olw: None,
            o_filename_base: o_filename_base,
            use_rolling: use_rolling,
            written_bytes: 0,
            roll_idx: roll_idx,
        };
        olw.mount_linewriter(&config.suffix, config.print_message);
        olw
    }

    fn mount_linewriter(&mut self, suffix: &Option<String>, print_message: bool) {
        match self.olw {
            None => {
                match self.o_filename_base {
                    Some(ref s_filename_base) => {
                        let filename = get_filename(s_filename_base, self.use_rolling, self.roll_idx, suffix);
                        let path = Path::new(&filename);
                        if print_message {
                            println!("Log is written to {}", path.display());
                        }
                        self.olw = Some(LineWriter::new(File::create(path.clone()).unwrap()));
                    },
                    None => {/* Folder is broken, logging not possible */}
                }
            },
            Some(_) => {/* we're all set to log */}
        }
    }
}

fn get_filename_base(s_directory: & String, o_discriminant: & Option<String>) -> String {
    let arg0 = env::args().next().unwrap();
    let progname = Path::new(&arg0).file_stem().unwrap().to_string_lossy();
    let mut filename = String::with_capacity(180).add(&s_directory).add("/").add(&progname);
    match * o_discriminant {
        Some(ref s_d) => {
            filename = filename.add(&format!("_{}", s_d));
        },
        None => {}
    }
    filename
}

fn get_filename(s_filename_base: &String,
                do_rolling: bool,
                roll_idx: usize,
                o_suffix: &Option<String>) -> String {
    let mut filename = String::with_capacity(180).add(&s_filename_base);
    if do_rolling {
        filename = filename.add(&format!("_{}", roll_idx));
    } else {
        filename = filename.add(&time::strftime("_%Y-%m-%d_%H-%M-%S",&time::now()).unwrap());
    }
    match o_suffix {
        & Some(ref suffix) => filename = filename.add(".").add(suffix),
        & None => {}
    }
    filename
}

fn get_filename_pattern(s_filename_base: & String, o_suffix: & Option<String>) -> String {
    let mut filename = String::with_capacity(180).add(&s_filename_base);
    filename = filename.add("_*");
    match o_suffix {
        & Some(ref suffix) => filename = filename.add(".").add(suffix),
        & None => {}
    }
    filename
}

fn get_next_roll_idx(s_filename_base: & String, o_suffix: & Option<String>) -> usize {
    let fn_pattern = get_filename_pattern(s_filename_base, o_suffix);
    let paths = glob(&fn_pattern);
    let mut roll_idx = 0;
    match paths {
        Err(e) => {
            panic!("Is this ({}) really a directory? Listing failed with {}", fn_pattern, e);
        },
        Ok(it) => {
            for globresult in it {
                match globresult {
                    Err(e) => println!("Ups - error occured: {}", e),
                    Ok(pathbuf) => {
                        println!("Found: {}", pathbuf.clone().into_os_string().to_string_lossy()); // FIXME delete this line
                        let filename = pathbuf.file_stem().unwrap().to_string_lossy();
                        let mut it = filename.rsplit("_r");
                        let idx: usize = it.next().unwrap().parse().unwrap_or(0);
                        println!("idx = {}", idx); // FIXME delete this line
                        roll_idx = max(roll_idx, idx);
                    }
                }
            }
        }
    }
    roll_idx+1
}


struct FlexiLogger{
    directives: Vec<LogDirective>,
    o_filter: Option<Regex>,
    amo_line_writer: Arc<Mutex<RefCell<OptLineWriter>>>,
    config: LogConfig
}
impl FlexiLogger {
    fn new( directives: Vec<LogDirective>,
            filter: Option<Regex>,
            config: LogConfig) -> FlexiLogger  {
        FlexiLogger {
                directives: directives,
                o_filter: filter,
                amo_line_writer: Arc::new(Mutex::new(RefCell::new(OptLineWriter::new(&config)))),
                config: config }
    }

    fn fl_enabled(&self, level: LogLevel, target: &str) -> bool {
        // Search for the longest match, the vector is assumed to be pre-sorted.
        for directive in self.directives.iter().rev() {
            match directive.name {
                Some(ref name) if !target.starts_with(&**name) => {},
                Some(..) | None => {
                    return level <= directive.level
                }
            }
        }
        false
    }
}

impl Log for FlexiLogger {
    fn enabled(&self, metadata: &LogMetadata) -> bool {
        self.fl_enabled(metadata.level(), metadata.target())
    }

    fn log(&self, record: &LogRecord) {
        if !Log::enabled(self, record.metadata()) {
            return;
        }

        if let Some(filter) = self.o_filter.as_ref() {
            if filter.is_match(&*record.args().to_string()) {
                return;
            }
        }

        let mut msg = (self.config.format)(record);
        msg.push('\n');
        if self.config.log_to_file {
            if self.config.duplicate_error && record.level() == LogLevel::Error
            || self.config.duplicate_info  && record.level() == LogLevel::Info {
                println!("{}",&record.args());
            }
            let msgb = msg.as_bytes();

            let amo_lw = self.amo_line_writer.clone();  // Arc<Mutex<RefCell<OptLineWriter>>>
            let mut mg_rc_olw = amo_lw.lock().unwrap(); // MutexGuard<RefCell<OptLineWriter>>
            let rc_olw = mg_rc_olw.deref_mut();         // &mut RefCell<OptLineWriter>
            let mut rm_olw = rc_olw.borrow_mut();       // RefMut<OptLineWriter>
            let olw: &mut OptLineWriter = rm_olw.deref_mut();

            if olw.use_rolling && (olw.written_bytes > self.config.rollover_size.unwrap()) {
                olw.olw = None;  // Hope that closes the previous lw
                olw.written_bytes = 0;
                olw.roll_idx += 1;
                olw.mount_linewriter(&self.config.suffix,  self.config.print_message);
            }
            match olw.olw {
                Some(ref mut lw) => {
                    &lw.write(msgb).unwrap_or_else( |e|{panic!("File logger: write failed with {}",e);} );
                    if olw.use_rolling {
                        olw.written_bytes += msgb.len();
                    }
                },
                None => {/* Folder is broken, logging not possible */}
            };
        } else {
            let _ = writeln!(&mut io::stderr(), "{}", msg );
        }
    }
}


/// Describes errors in the initialization of flexi_logger.
#[derive(Debug)]
pub struct FlexiLoggerError {
    message: &'static str
}
impl FlexiLoggerError {
    pub fn new(s: &'static str) -> FlexiLoggerError {
        FlexiLoggerError {message: s}
    }
}
impl fmt::Display for  FlexiLoggerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Allows influencing the behavior of flexi_logger.
pub struct LogConfig {
    /// If `true`, the log is written to a file. Default is `false`, the log is then
    /// written to stderr.
    /// If `true`, a new file in the current directory is created and written to.
    /// The name of the file is chosen as '\<program_name\>\_\<date\>\_\<time\>.<suffix>',
    ///  e.g. `myprog_2015-07-08_10-44-11.log`
    pub log_to_file: bool,

    /// If `true` (which is default), and if `log_to_file` is `true`,
    /// the name of the logfile is documented in a message to stdout.
    pub print_message: bool,

    /// If `true` (which is default), and if `log_to_file` is `true`,
    /// all logged error messages are duplicated to stdout.
    pub duplicate_error: bool,

    /// If `true` (which is default), and if `log_to_file` is `true`,
    /// all logged warning and info messages are also duplicated to stdout.
    pub duplicate_info: bool,

    /// Allows providing a custom logline format; default is ```flexi_logger::default_format```.
    pub format: fn(&LogRecord) -> String,

    /// Allows specifying a directory in which the log files are created
    pub directory: Option<String>,

    /// Allows specifying the filesystem suffix of the log files
    pub suffix: Option<String>,

    /// Allows specifying a rollover of log files if a certain size is exceeded; the rollover will happen when
    /// the specified file size is reached or exceeded.
    pub rollover_size: Option<usize>,

    /// Allows specifying an optional part of the log file name that is inserted after the program name
    pub o_discriminant: Option<String>,
}
impl LogConfig {
    /// initializes with
    ///
    /// *  log_to_file = false,
    /// *  print_message = true,
    /// *  duplicate_error = true,
    /// *  duplicate_info = false,
    /// *  format = flexi_logger::default_format,
    /// *  no directory (log files are created where the program was started),
    /// *  no rollover: log file grows indefinitely
    /// *  no discriminant: log file name consists only of progname, date or roll_idx,  and suffix.
    /// *  suffix =  "log".

    pub fn new() -> LogConfig {
        LogConfig {
            log_to_file: false,
            print_message: true,
            duplicate_error: true,
            duplicate_info: false,
            format: default_format,
            directory: None,
            rollover_size: None,
            o_discriminant: None,
            suffix: Some("log".to_string()),
        }
    }
}

/// A logline-formatter that produces lines like <br>
/// ```INFO [my_prog::some_submodel] Task successfully read from conf.json```
pub fn default_format(record: &LogRecord) -> String {
    format!( "{} [{}] {}", record.level(), record.location().module_path(), record.args() )
}

/// A logline-formatter that produces lines like <br>
/// ```[2015-07-08 12:12:32:639785] INFO [my_prog::some_submodel] src/some_submodel.rs:26: Task successfully read from conf.json```
#[allow(unused)]
pub fn detailed_format(record: &LogRecord) -> String {
    let timespec = time::get_time(); // high-precision now
    let tm = time::at(timespec);     // formattable. but low-precision now
    let mut time: String = time::strftime("%Y-%m-%d %H:%M:%S:", &tm).unwrap();
    // ugly code to format milli and micro seconds
    let tmp = 1000000000 + timespec.nsec;
    let mut s = tmp.to_string();
    s.remove(9);s.remove(8);s.remove(7);s.remove(0);
    time = time.add(&s);
    format!( "[{}] {} [{}] {}:{}: {}",
                &time,
                record.level(),
                record.location().module_path(),
                record.location().file(),
                record.location().line(),
                &record.args())
}

struct LogDirective {
    name: Option<String>,
    level: LogLevelFilter,
}

/// Initializes the flexi_logger to your needs, and the global logger with flexi_logger.
///
/// Note: this should be called early in the execution of a Rust program. The
/// global logger may only be initialized once, subsequent initialization attempts
/// will return an error.
///
/// ## Configuration
///
/// See [LogConfig](struct.LogConfig.html) for most of the initialization options.
///
/// ## Log Level Specification
///
/// Specifying the log levels that you really want to see in a specific program run
/// can be done in the syntax defined by
/// [env_logger -> enabling logging](http://rust-lang.github.io/log/env_logger/#enabling-logging)
/// (from where this functionality was ruthlessly copied).
/// You can hand over the desired log-level-specification as an
/// initialization parameter to flexi_logger, or, if you don't do so,
/// with the environment variable RUST_LOG (as with env_logger).
/// Since using environment variables is on Windows not as comfortable as on linux,
/// you might consider using e.g. a docopt option for specifying the
/// log-Level-specification on the command line of your program.
///
///
/// ## Examples
///
/// ### Use defaults only
///
/// If you initialize flexi_logger with default settings, then it behaves like env_logger:
///
/// ```
/// use flexi_logger::{init,LogConfig};
///
/// init(LogConfig::new(), None).unwrap();
/// ```
///
/// ### Write to files, use a detailed log-line format
///
/// Here we configure flexi_logger to write log entries with fine-grained
/// time and location info into a log file in folder "log_files",
/// and we provide the loglevel-specification programmatically
/// as a ```Some<String>```, which might come in this form from what docopt provides,
/// if you have a command-line option ```--loglevelspec```:
///
/// ```
/// use flexi_logger::{detailed_format,init,LogConfig};
///
/// init( LogConfig { log_to_file: true,
///                   directory: "log_files",
///                   format: detailed_format,
///                    .. LogConfig::new() },
///       args.flag_loglevelspec )
/// .unwrap_or_else(|e|{panic!("Logger initialization failed with {}",e)});
/// ```
///
/// # Failures
///
/// Init returns a FlexiLoggerError, if it is supposed to write to an output file
/// but the file cannot be opened, e.g. because of operating system issues.
///
pub fn init(config: LogConfig, loglevelspec: Option<String>) -> Result<(),FlexiLoggerError> {
    log::set_logger( |max_level| {
        let (mut directives, filter) =
            match loglevelspec {
                Some(ref llspec) => {let spec: &str = llspec; parse_logging_spec(&spec)},
                None => {
                    match env::var("RUST_LOG") {
                        Ok(spec) => parse_logging_spec(&spec),
                        Err(..) => (vec![LogDirective { name: None, level: LogLevelFilter::Error }], None),
                    }
                }
            };

        // Sort the provided directives by length of their name, this allows a
        // little more efficient lookup at runtime.
        directives.sort_by(|a, b| {
            let alen = a.name.as_ref().map(|a| a.len()).unwrap_or(0);
            let blen = b.name.as_ref().map(|b| b.len()).unwrap_or(0);
            alen.cmp(&blen)
        });

        let level = {
            let max = directives.iter().map(|d| d.level).max();
            max.unwrap_or(LogLevelFilter::Off)
        };
        max_level.set(level);
        Box::new(FlexiLogger::new(directives,filter,config))
    }).map_err(|_|{FlexiLoggerError::new("Logger initialization failed")})
}

/// Parse a logging specification string (e.g: "crate1,crate2::mod3,crate3::x=error/foo")
/// and return a vector with log directives.
fn parse_logging_spec(spec: &str) -> (Vec<LogDirective>, Option<Regex>) {
    let mut dirs = Vec::new();

    let mut parts = spec.split('/');
    let mods = parts.next();
    let filter = parts.next();
    if parts.next().is_some() {
        print_err!("warning: invalid logging spec '{}', ignoring it (too many '/'s)", spec);
        return (dirs, None);
    }
    mods.map(|m| { for s in m.split(',') {
        if s.len() == 0 { continue }
        let mut parts = s.split('=');
        let (log_level, name) = match (parts.next(), parts.next().map(|s| s.trim()), parts.next()) {
            (Some(part0), None, None) => {
                // if the single argument is a log-level string or number, treat that as a global fallback
                match part0.parse() {
                    Ok(num) => (num, None),
                    Err(_) => (LogLevelFilter::max(), Some(part0)),
                }
            }
            (Some(part0), Some(""), None) => (LogLevelFilter::max(), Some(part0)),
            (Some(part0), Some(part1), None) => {
                match part1.parse() {
                    Ok(num) => (num, Some(part0)),
                    _ => {
                        print_err!("warning: invalid logging spec '{}', ignoring it", part1);
                        continue
                    }
                }
            },
            _ => {
                print_err!("warning: invalid logging spec '{}', ignoring it", s);
                continue
            }
        };
        dirs.push(LogDirective {
            name: name.map(|s| s.to_string()),
            level: log_level,
        });
    }});

    let filter = filter.map_or(None, |filter| {
        match Regex::new(filter) {
            Ok(re) => Some(re),
            Err(e) => {
                print_err!("warning: invalid regex filter - {}", e);
                None
            }
        }
    });

    return (dirs, filter);
}
