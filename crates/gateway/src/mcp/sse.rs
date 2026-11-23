//! Reads the `data` of server-sent events out of a byte stream.

#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buffer: Vec<u8>,
    data: Option<String>,
}

impl SseParser {
    /// Feeds a chunk and returns the data of every event it completed.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches('\n').trim_end_matches('\r');
            if let Some(event) = self.line(line) {
                events.push(event);
            }
        }
        events
    }

    /// The event the stream ended in the middle of, if any.
    pub(crate) fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buffer);
        let rest = String::from_utf8_lossy(&rest);
        let rest = rest.trim_end_matches('\r');
        if !rest.is_empty() {
            self.line(rest);
        }
        self.data.take()
    }

    fn line(&mut self, line: &str) -> Option<String> {
        if line.is_empty() {
            return self.data.take();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        if field == "data" {
            let value = value.strip_prefix(' ').unwrap_or(value);
            match &mut self.data {
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
                None => self.data = Some(value.to_string()),
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_ends_at_a_blank_line() {
        let mut parser = SseParser::default();

        let events = parser.push(b"event: message\nid: 1\ndata: {\"a\":1}\n\ndata: x\n");

        assert_eq!(events, vec![r#"{"a":1}"#.to_string()]);
        assert_eq!(parser.push(b"\n"), vec!["x".to_string()]);
    }

    #[test]
    fn data_lines_of_one_event_join_with_newlines() {
        let mut parser = SseParser::default();

        let events = parser.push(b"data: one\r\ndata:two\r\n\r\n");

        assert_eq!(events, vec!["one\ntwo".to_string()]);
    }

    #[test]
    fn comments_and_chunk_boundaries_do_not_break_events() {
        let mut parser = SseParser::default();

        let rest = "ta: 日\n\n".as_bytes();

        let mut events = parser.push(b":keep-alive\n\nda");
        events.extend(parser.push(&rest[..5]));
        events.extend(parser.push(&rest[5..]));

        assert_eq!(events, vec!["日".to_string()]);
    }

    #[test]
    fn an_event_cut_off_by_the_end_of_the_stream_is_still_read() {
        let mut parser = SseParser::default();

        assert!(parser.push(b"data: last").is_empty());
        assert_eq!(parser.finish(), Some("last".to_string()));
    }
}
