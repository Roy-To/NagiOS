//! Nagi Slides host native format v1: little-endian, length-prefixed UTF-8,
//! canonical sorted maps/issued IDs, explicit enum tags, no extension guessing.
use crate::*;
use std::collections::{BTreeMap, BTreeSet};
pub const MAGIC: &[u8; 8] = b"NAGISLD\0";
pub const VERSION: u16 = 1;

struct Writer {
    bytes: Option<Vec<u8>>,
    len: usize,
    bound: usize,
}
macro_rules! write_scalar {
    ($name:ident, $ty:ty) => {
        fn $name(&mut self, n: $ty) -> Result<()> {
            self.put(&n.to_le_bytes())
        }
    };
}
impl Writer {
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        let len = self
            .len
            .checked_add(bytes.len())
            .ok_or(Error::LimitExceeded)?;
        if len > self.bound {
            return Err(Error::LimitExceeded);
        }
        if let Some(output) = &mut self.bytes {
            output.extend_from_slice(bytes);
        }
        self.len = len;
        Ok(())
    }
    write_scalar!(u8, u8);
    write_scalar!(u16, u16);
    write_scalar!(u32, u32);
    write_scalar!(u64, u64);
    write_scalar!(i64, i64);
    write_scalar!(i32, i32);
    fn count(&mut self, n: usize) -> Result<()> {
        self.u32(u32::try_from(n).map_err(|_| Error::LimitExceeded)?)
    }
    fn text(&mut self, s: &str) -> Result<()> {
        self.count(s.len())?;
        self.put(s.as_bytes())
    }
    fn optional_id(&mut self, id: Option<ObjectId>) -> Result<()> {
        self.u8(u8::from(id.is_some()))?;
        if let Some(id) = id {
            self.u64(id.0)?;
        }
        Ok(())
    }
    fn optional_text(&mut self, text: &Option<String>) -> Result<()> {
        self.u8(u8::from(text.is_some()))?;
        if let Some(text) = text {
            self.text(text)?;
        }
        Ok(())
    }
    fn source(&mut self, source: &SourceReference) -> Result<()> {
        self.u64(source.resource.0)?;
        self.optional_id(source.object)?;
        self.u64(source.revision.0)?;
        self.u8(match source.kind {
            SourceKind::SheetsChart => 0,
            SourceKind::SheetsTable => 1,
            SourceKind::Writer => 2,
            SourceKind::Notes => 3,
            SourceKind::Albert => 4,
            SourceKind::Other => 5,
        })?;
        self.text(&source.label)?;
        self.optional_text(&source.locator)
    }
    fn object(&mut self, object: &Object) -> Result<()> {
        self.u64(object.id.0)?;
        let g = object.geometry;
        self.i64(g.x)?;
        self.i64(g.y)?;
        self.i64(g.size.width)?;
        self.i64(g.size.height)?;
        self.u32(g.rotation_millidegrees)?;
        self.i32(g.z_order)?;
        match &object.content {
            Content::Text(t) => {
                self.u8(0)?;
                self.text(t)?;
            }
            Content::Shape(t) => {
                self.u8(1)?;
                self.text(t)?;
            }
            Content::EmbeddedReference => self.u8(2)?,
        }
        self.optional_text(&object.theme_token)?;
        self.u8(u8::from(object.source.is_some()))?;
        if let Some(source) = &object.source {
            self.source(source)?;
        }
        self.optional_id(object.target)
    }
    fn theme(&mut self, theme: &Theme) -> Result<()> {
        self.u64(theme.id.0)?;
        self.i64(theme.slide_size.width)?;
        self.i64(theme.slide_size.height)?;
        self.count(theme.fonts.len())?;
        for (key, font) in &theme.fonts {
            self.text(key)?;
            self.text(font)?;
        }
        self.count(theme.colors.len())?;
        for (key, color) in &theme.colors {
            self.text(key)?;
            self.u32(*color)?;
        }
        Ok(())
    }
    fn slide(&mut self, slide: &Slide) -> Result<()> {
        self.u64(slide.id.0)?;
        self.u64(slide.layout.0)?;
        self.text(&slide.notes)?;
        self.count(slide.objects.len())?;
        for object in &slide.objects {
            self.object(object)?;
        }
        Ok(())
    }
}
fn write_presentation(w: &mut Writer, p: &Presentation) -> Result<()> {
    w.put(MAGIC)?;
    w.u16(VERSION)?;
    w.u64(p.id.0)?;
    w.u64(p.revision.0)?;
    w.text(&p.title)?;
    w.count(p.issued.len())?;
    for id in &p.issued {
        w.u64(id.0)?;
    }
    w.theme(&p.theme)?;
    w.count(p.layouts.len())?;
    for layout in &p.layouts {
        w.u64(layout.id.0)?;
        w.u8(match layout.kind {
            LayoutKind::TitleContent => 0,
            LayoutKind::SectionHeader => 1,
            LayoutKind::TitleOnly => 2,
            LayoutKind::Blank => 3,
            LayoutKind::Custom => 4,
        })?;
        w.text(&layout.name)?;
    }
    w.count(p.slides.len())?;
    for slide in &p.slides {
        w.slide(slide)?;
    }
    Ok(())
}
pub(crate) fn encode_validated(p: &Presentation, max_bytes: usize) -> Result<Vec<u8>> {
    let mut w = Writer {
        bytes: Some(vec![]),
        len: 0,
        bound: max_bytes,
    };
    write_presentation(&mut w, p)?;
    w.bytes.ok_or(Error::MalformedNative)
}
/// Exact native size through the same writer, without a byte buffer or cloning.
fn measure(bound: usize, write: impl FnOnce(&mut Writer) -> Result<()>) -> Result<usize> {
    let mut w = Writer {
        bytes: None,
        len: 0,
        bound,
    };
    write(&mut w)?;
    Ok(w.len)
}
pub(crate) fn encoded_len(p: &Presentation, bound: usize) -> Result<usize> {
    measure(bound, |w| write_presentation(w, p))
}
pub(crate) fn slide_len(slide: &Slide, bound: usize) -> Result<usize> {
    measure(bound, |w| w.slide(slide))
}
pub(crate) fn object_len(object: &Object, bound: usize) -> Result<usize> {
    measure(bound, |w| w.object(object))
}
pub(crate) fn theme_len(theme: &Theme, bound: usize) -> Result<usize> {
    measure(bound, |w| w.theme(theme))
}
pub(crate) fn text_len(text: &str, bound: usize) -> Result<usize> {
    measure(bound, |w| w.text(text))
}
pub fn encode(p: &Presentation, limits: Limits) -> Result<Vec<u8>> {
    p.validate(limits)?;
    encode_validated(p, limits.max_bytes)
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
    limits: Limits,
}
macro_rules! read_scalar {
    ($name:ident, $ty:ty, $width:expr) => {
        fn $name(&mut self) -> Result<$ty> {
            Ok(<$ty>::from_le_bytes(
                self.take($width)?
                    .try_into()
                    .map_err(|_| Error::MalformedNative)?,
            ))
        }
    };
}
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.cursor.checked_add(len).ok_or(Error::MalformedNative)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(Error::MalformedNative)?;
        self.cursor = end;
        Ok(value)
    }
    read_scalar!(u8, u8, 1);
    read_scalar!(u16, u16, 2);
    read_scalar!(u32, u32, 4);
    read_scalar!(u64, u64, 8);
    read_scalar!(i64, i64, 8);
    read_scalar!(i32, i32, 4);
    fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(Error::LimitExceeded);
        }
        Ok(n)
    }
    fn text(&mut self) -> Result<String> {
        let len = self.count(self.limits.max_text_bytes)?;
        let text = std::str::from_utf8(self.take(len)?).map_err(|_| Error::MalformedNative)?;
        Ok(text.to_owned())
    }
    fn present(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::MalformedNative),
        }
    }
    fn optional_id(&mut self) -> Result<Option<ObjectId>> {
        if self.present()? {
            Ok(Some(ObjectId(self.u64()?)))
        } else {
            Ok(None)
        }
    }
    fn optional_text(&mut self) -> Result<Option<String>> {
        if self.present()? {
            Ok(Some(self.text()?))
        } else {
            Ok(None)
        }
    }
    fn source(&mut self) -> Result<SourceReference> {
        let resource = ObjectId(self.u64()?);
        let object = self.optional_id()?;
        let revision = RevisionId(self.u64()?);
        let kind = match self.u8()? {
            0 => SourceKind::SheetsChart,
            1 => SourceKind::SheetsTable,
            2 => SourceKind::Writer,
            3 => SourceKind::Notes,
            4 => SourceKind::Albert,
            5 => SourceKind::Other,
            _ => return Err(Error::MalformedNative),
        };
        Ok(SourceReference {
            resource,
            object,
            revision,
            kind,
            label: self.text()?,
            locator: self.optional_text()?,
        })
    }
    fn object(&mut self) -> Result<Object> {
        let id = ObjectId(self.u64()?);
        let geometry = Geometry {
            x: self.i64()?,
            y: self.i64()?,
            size: LogicalSize {
                width: self.i64()?,
                height: self.i64()?,
            },
            rotation_millidegrees: self.u32()?,
            z_order: self.i32()?,
        };
        let content = match self.u8()? {
            0 => Content::Text(self.text()?),
            1 => Content::Shape(self.text()?),
            2 => Content::EmbeddedReference,
            _ => return Err(Error::MalformedNative),
        };
        let theme_token = self.optional_text()?;
        let source = if self.present()? {
            Some(self.source()?)
        } else {
            None
        };
        Ok(Object {
            id,
            geometry,
            content,
            theme_token,
            source,
            target: self.optional_id()?,
        })
    }
}
/// Validate byte/count bounds before allocating; unknown versions never guess v1.
pub fn decode(bytes: &[u8], limits: Limits) -> Result<Presentation> {
    limits.validate()?;
    if bytes.len() > limits.max_bytes {
        return Err(Error::LimitExceeded);
    }
    let mut r = Reader {
        bytes,
        cursor: 0,
        limits,
    };
    if r.take(MAGIC.len())? != MAGIC {
        return Err(Error::MalformedNative);
    }
    let version = r.u16()?;
    if version != VERSION {
        return Err(Error::UnknownVersion(version));
    }
    let id = ObjectId(r.u64()?);
    let revision = RevisionId(r.u64()?);
    let title = r.text()?;
    let mut issued = BTreeSet::new();
    for _ in 0..r.count(limits.max_ids)? {
        if !issued.insert(ObjectId(r.u64()?)) {
            return Err(Error::MalformedNative);
        }
    }
    let theme_id = ObjectId(r.u64()?);
    let slide_size = LogicalSize {
        width: r.i64()?,
        height: r.i64()?,
    };
    let mut fonts = BTreeMap::new();
    for _ in 0..r.count(64)? {
        if fonts.insert(r.text()?, r.text()?).is_some() {
            return Err(Error::MalformedNative);
        }
    }
    let mut colors = BTreeMap::new();
    for _ in 0..r.count(256)? {
        if colors.insert(r.text()?, r.u32()?).is_some() {
            return Err(Error::MalformedNative);
        }
    }
    let theme = Theme {
        id: theme_id,
        slide_size,
        fonts,
        colors,
    };
    let mut layouts = vec![];
    for _ in 0..r.count(limits.max_ids)? {
        let id = ObjectId(r.u64()?);
        let kind = match r.u8()? {
            0 => LayoutKind::TitleContent,
            1 => LayoutKind::SectionHeader,
            2 => LayoutKind::TitleOnly,
            3 => LayoutKind::Blank,
            4 => LayoutKind::Custom,
            _ => return Err(Error::MalformedNative),
        };
        layouts.push(Layout {
            id,
            kind,
            name: r.text()?,
        });
    }
    let mut slides = vec![];
    let mut object_count = 0usize;
    for _ in 0..r.count(limits.max_slides)? {
        let id = ObjectId(r.u64()?);
        let layout = ObjectId(r.u64()?);
        let notes = r.text()?;
        let count = r.count(limits.max_objects_per_slide)?;
        object_count = object_count
            .checked_add(count)
            .ok_or(Error::LimitExceeded)?;
        if object_count > limits.max_ids {
            return Err(Error::LimitExceeded);
        }
        let mut objects = vec![];
        for _ in 0..count {
            objects.push(r.object()?);
        }
        slides.push(Slide {
            id,
            layout,
            notes,
            objects,
        });
    }
    if r.cursor != bytes.len() {
        return Err(Error::MalformedNative);
    }
    let p = Presentation {
        id,
        revision,
        title,
        theme,
        layouts,
        slides,
        issued,
    };
    p.validate(limits)?;
    if encode_validated(&p, limits.max_bytes)? != bytes {
        return Err(Error::MalformedNative);
    }
    Ok(p)
}
