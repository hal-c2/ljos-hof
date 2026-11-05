//! The values in a PulseAudio control packet. Each is a tag byte that says
//! what it is, then the value, big-endian.

/// What a packet held that was not what its command takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a PulseAudio packet that does not hold what its command takes")]
pub struct Malformed;

const STRING: u8 = b't';
const NULL_STRING: u8 = b'N';
const U32: u8 = b'L';
const U8: u8 = b'B';
const U64: u8 = b'R';
const S64: u8 = b'r';
const SAMPLE_SPEC: u8 = b'a';
const ARBITRARY: u8 = b'x';
const TRUE: u8 = b'1';
const FALSE: u8 = b'0';
const TIMEVAL: u8 = b'T';
const USEC: u8 = b'U';
const CHANNEL_MAP: u8 = b'm';
const CVOLUME: u8 = b'v';
const PROPLIST: u8 = b'P';
const VOLUME: u8 = b'V';
const FORMAT_INFO: u8 = b'f';

/// How a stream's samples are laid out: their format, how many channels,
/// and how many frames a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleSpec {
    pub format: u8,
    pub channels: u8,
    pub rate: u32,
}

/// Names and values: what a client or stream says of itself.
pub type Proplist = Vec<(String, Vec<u8>)>;

/// Reads the values of a packet in turn.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if self.bytes.len() < n {
            return Err(Malformed);
        }
        let (taken, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(taken)
    }

    fn tag(&mut self, tag: u8) -> Result<(), Malformed> {
        (self.take(1)?[0] == tag).then_some(()).ok_or(Malformed)
    }

    fn be32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("four")))
    }

    pub fn u32(&mut self) -> Result<u32, Malformed> {
        self.tag(U32)?;
        self.be32()
    }

    pub fn u8(&mut self) -> Result<u8, Malformed> {
        self.tag(U8)?;
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool, Malformed> {
        match self.take(1)?[0] {
            TRUE => Ok(true),
            FALSE => Ok(false),
            _ => Err(Malformed),
        }
    }

    pub fn string(&mut self) -> Result<Option<String>, Malformed> {
        match self.take(1)?[0] {
            NULL_STRING => Ok(None),
            STRING => {
                let end = self.bytes.iter().position(|&b| b == 0).ok_or(Malformed)?;
                let s = String::from_utf8_lossy(self.take(end)?).into_owned();
                self.take(1)?;
                Ok(Some(s))
            }
            _ => Err(Malformed),
        }
    }

    pub fn arbitrary(&mut self) -> Result<&'a [u8], Malformed> {
        self.tag(ARBITRARY)?;
        let len = self.be32()? as usize;
        self.take(len)
    }

    pub fn sample_spec(&mut self) -> Result<SampleSpec, Malformed> {
        self.tag(SAMPLE_SPEC)?;
        let b = self.take(2)?;
        let (format, channels) = (b[0], b[1]);
        Ok(SampleSpec {
            format,
            channels,
            rate: self.be32()?,
        })
    }

    pub fn channel_map(&mut self) -> Result<Vec<u8>, Malformed> {
        self.tag(CHANNEL_MAP)?;
        let n = self.take(1)?[0] as usize;
        Ok(self.take(n)?.to_vec())
    }

    pub fn cvolume(&mut self) -> Result<Vec<u32>, Malformed> {
        self.tag(CVOLUME)?;
        let n = self.take(1)?[0] as usize;
        (0..n).map(|_| self.be32()).collect()
    }

    pub fn timeval(&mut self) -> Result<(u32, u32), Malformed> {
        self.tag(TIMEVAL)?;
        Ok((self.be32()?, self.be32()?))
    }

    pub fn proplist(&mut self) -> Result<Proplist, Malformed> {
        self.tag(PROPLIST)?;
        let mut props = Vec::new();
        while let Some(key) = self.string()? {
            let len = self.u32()? as usize;
            let value = self.arbitrary()?;
            if value.len() != len {
                return Err(Malformed);
            }
            props.push((key, value.to_vec()));
        }
        Ok(props)
    }

    pub fn format_info(&mut self) -> Result<(u8, Proplist), Malformed> {
        self.tag(FORMAT_INFO)?;
        Ok((self.u8()?, self.proplist()?))
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Writes the values of a packet in turn.
#[derive(Debug, Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.bytes.push(U32);
        self.bytes.extend(v.to_be_bytes());
        self
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.bytes.extend([U8, v]);
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.bytes.push(U64);
        self.bytes.extend(v.to_be_bytes());
        self
    }

    pub fn s64(&mut self, v: i64) -> &mut Self {
        self.bytes.push(S64);
        self.bytes.extend(v.to_be_bytes());
        self
    }

    pub fn usec(&mut self, v: u64) -> &mut Self {
        self.bytes.push(USEC);
        self.bytes.extend(v.to_be_bytes());
        self
    }

    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.bytes.push(if v { TRUE } else { FALSE });
        self
    }

    pub fn string(&mut self, v: Option<&str>) -> &mut Self {
        match v {
            None => self.bytes.push(NULL_STRING),
            Some(s) => {
                self.bytes.push(STRING);
                // A string can't hold a NUL; cut it there.
                self.bytes.extend(s.bytes().take_while(|&b| b != 0));
                self.bytes.push(0);
            }
        }
        self
    }

    pub fn sample_spec(&mut self, v: SampleSpec) -> &mut Self {
        self.bytes.extend([SAMPLE_SPEC, v.format, v.channels]);
        self.bytes.extend(v.rate.to_be_bytes());
        self
    }

    pub fn channel_map(&mut self, v: &[u8]) -> &mut Self {
        self.bytes.extend([CHANNEL_MAP, v.len() as u8]);
        self.bytes.extend(v);
        self
    }

    pub fn cvolume(&mut self, v: &[u32]) -> &mut Self {
        self.bytes.extend([CVOLUME, v.len() as u8]);
        for c in v {
            self.bytes.extend(c.to_be_bytes());
        }
        self
    }

    pub fn volume(&mut self, v: u32) -> &mut Self {
        self.bytes.push(VOLUME);
        self.bytes.extend(v.to_be_bytes());
        self
    }

    pub fn timeval(&mut self, (sec, usec): (u32, u32)) -> &mut Self {
        self.bytes.push(TIMEVAL);
        self.bytes.extend(sec.to_be_bytes());
        self.bytes.extend(usec.to_be_bytes());
        self
    }

    /// `props`, each a string held with its terminating NUL, as
    /// PulseAudio holds them.
    pub fn proplist(&mut self, props: &[(&str, &str)]) -> &mut Self {
        self.bytes.push(PROPLIST);
        for (key, value) in props {
            let mut data = value.as_bytes().to_vec();
            data.push(0);
            self.string(Some(key)).u32(data.len() as u32);
            self.bytes.push(ARBITRARY);
            self.bytes.extend((data.len() as u32).to_be_bytes());
            self.bytes.extend(data);
        }
        self.string(None)
    }

    pub fn format_info(&mut self, encoding: u8, props: &[(&str, &str)]) -> &mut Self {
        self.bytes.push(FORMAT_INFO);
        self.u8(encoding).proplist(props)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back() {
        let spec = SampleSpec {
            format: 3,
            channels: 2,
            rate: 44_100,
        };
        let mut w = Writer::new();
        w.u32(7)
            .string(Some("paplay"))
            .string(None)
            .bool(true)
            .sample_spec(spec)
            .channel_map(&[1, 2])
            .cvolume(&[0x10000, 0x8000])
            .proplist(&[("application.name", "paplay")])
            .format_info(1, &[])
            .timeval((5, 6))
            .u8(9);
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u32(), Ok(7));
        assert_eq!(r.string(), Ok(Some("paplay".into())));
        assert_eq!(r.string(), Ok(None));
        assert_eq!(r.bool(), Ok(true));
        assert_eq!(r.sample_spec(), Ok(spec));
        assert_eq!(r.channel_map(), Ok(vec![1, 2]));
        assert_eq!(r.cvolume(), Ok(vec![0x10000, 0x8000]));
        assert_eq!(
            r.proplist(),
            Ok(vec![("application.name".into(), b"paplay\0".to_vec())])
        );
        assert_eq!(r.format_info(), Ok((1, vec![])));
        assert_eq!(r.timeval(), Ok((5, 6)));
        assert_eq!(r.u8(), Ok(9));
        assert!(r.is_empty());
    }

    #[test]
    fn a_value_of_another_kind_or_cut_short_is_malformed() {
        let mut w = Writer::new();
        w.u32(7);
        let bytes = w.into_bytes();
        assert_eq!(Reader::new(&bytes).bool(), Err(Malformed));
        assert_eq!(Reader::new(&bytes[..3]).u32(), Err(Malformed));
        assert_eq!(Reader::new(b"tno end").string(), Err(Malformed));
    }
}
