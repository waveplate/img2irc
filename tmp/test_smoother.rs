        use std::fs::OpenOptions;
        use std::io::Write;
        
        pub fn debug_log(msg: &str) {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open("/tmp/img2irc_debug.log") {
                let _ = writeln!(file, "{}", msg);
            }
        }
