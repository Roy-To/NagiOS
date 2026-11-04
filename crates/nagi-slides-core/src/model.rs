use crate::{Error, ObjectId, Result, RevisionId};
use std::collections::{BTreeMap, BTreeSet};

/// Logical millipoints, unrelated to a display's pixels or DPI.
pub const MAX_LOGICAL: i64 = 1_000_000_000;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalSize {
    pub width: i64,
    pub height: i64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Geometry {
    pub x: i64,
    pub y: i64,
    pub size: LogicalSize,
    /// Canonical clockwise angle: 0 <= angle < 360000.
    pub rotation_millidegrees: u32,
    pub z_order: i32,
}
impl LogicalSize {
    pub fn validate(self) -> Result<()> {
        if !(1..=MAX_LOGICAL).contains(&self.width) || !(1..=MAX_LOGICAL).contains(&self.height) {
            return Err(Error::InvalidGeometry);
        }
        Ok(())
    }
}
impl Geometry {
    pub fn validate(self) -> Result<()> {
        self.size.validate()?;
        for (position, length) in [(self.x, self.size.width), (self.y, self.size.height)] {
            if !(-MAX_LOGICAL..=MAX_LOGICAL).contains(&position)
                || position
                    .checked_add(length)
                    .is_none_or(|end| end > MAX_LOGICAL)
            {
                return Err(Error::InvalidGeometry);
            }
        }
        if self.rotation_millidegrees >= 360_000 {
            return Err(Error::InvalidGeometry);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Entire canonical native snapshot, including IDs, tombstones and metadata.
    pub max_bytes: usize,
    pub max_ids: usize,
    pub max_slides: usize,
    pub max_objects_per_slide: usize,
    pub max_text_bytes: usize,
    pub max_operations: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 1_048_576,
            max_ids: 4096,
            max_slides: 256,
            max_objects_per_slide: 256,
            max_text_bytes: 65_536,
            max_operations: 256,
        }
    }
}
impl Limits {
    pub(crate) fn validate(self) -> Result<()> {
        if !(1..=16_777_216).contains(&self.max_bytes)
            || !(1..=65_536).contains(&self.max_ids)
            || !(1..=4096).contains(&self.max_slides)
            || !(1..=4096).contains(&self.max_objects_per_slide)
            || self.max_text_bytes == 0
            || self.max_text_bytes > self.max_bytes
            || !(1..=4096).contains(&self.max_operations)
        {
            return Err(Error::LimitExceeded);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Theme {
    pub id: ObjectId,
    pub slide_size: LogicalSize,
    pub fonts: BTreeMap<String, String>,
    /// RGBA tokens, sorted by name in native serialization.
    pub colors: BTreeMap<String, u32>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutKind {
    TitleContent,
    SectionHeader,
    TitleOnly,
    Blank,
    Custom,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layout {
    pub id: ObjectId,
    pub kind: LayoutKind,
    pub name: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    SheetsChart,
    SheetsTable,
    Writer,
    Notes,
    Albert,
    Other,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceReference {
    pub resource: ObjectId,
    pub object: Option<ObjectId>,
    pub revision: RevisionId,
    pub kind: SourceKind,
    pub label: String,
    /// Untrusted provenance locator, never executed or fetched by this core.
    pub locator: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Content {
    Text(String),
    Shape(String),
    EmbeddedReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Object {
    pub id: ObjectId,
    pub geometry: Geometry,
    pub content: Content,
    pub theme_token: Option<String>,
    pub source: Option<SourceReference>,
    /// Internal navigation reference to a live slide/object in this presentation.
    pub target: Option<ObjectId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Slide {
    pub id: ObjectId,
    pub layout: ObjectId,
    pub notes: String,
    pub objects: Vec<Object>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Presentation {
    pub(crate) id: ObjectId,
    pub(crate) revision: RevisionId,
    pub(crate) title: String,
    pub(crate) theme: Theme,
    pub(crate) layouts: Vec<Layout>,
    pub(crate) slides: Vec<Slide>,
    /// Issued canonical IDs, including deletions; persistent no-reuse tombstones.
    pub(crate) issued: BTreeSet<ObjectId>,
}
pub(crate) fn valid_id(id: ObjectId) -> Result<()> {
    if id.0 == 0 || id.0 == u64::MAX {
        Err(Error::InvalidId)
    } else {
        Ok(())
    }
}
pub(crate) fn valid_revision(r: RevisionId) -> Result<()> {
    RevisionId::new(r.0)
        .map(|_| ())
        .map_err(|_| Error::InvalidRevision)
}
pub(crate) fn text(s: &str, limits: Limits) -> Result<()> {
    if s.len() > limits.max_text_bytes {
        return Err(Error::LimitExceeded);
    }
    if s.contains('\0') {
        return Err(Error::InvalidText);
    }
    Ok(())
}
impl Presentation {
    pub fn new(id: ObjectId, theme: Theme, layouts: Vec<Layout>, limits: Limits) -> Result<Self> {
        limits.validate()?;
        if layouts.len() > limits.max_ids {
            return Err(Error::LimitExceeded);
        }
        let mut value = Self {
            id,
            revision: RevisionId(1),
            title: String::new(),
            theme,
            layouts,
            slides: vec![],
            issued: BTreeSet::new(),
        };
        value.issue(id, limits)?;
        value.issue(value.theme.id, limits)?;
        for id in value.layouts.iter().map(|l| l.id).collect::<Vec<_>>() {
            value.issue(id, limits)?;
        }
        value.validate(limits)?;
        Ok(value)
    }
    pub fn id(&self) -> ObjectId {
        self.id
    }
    pub fn revision(&self) -> RevisionId {
        self.revision
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn theme(&self) -> &Theme {
        &self.theme
    }
    pub fn layouts(&self) -> &[Layout] {
        &self.layouts
    }
    pub fn slides(&self) -> &[Slide] {
        &self.slides
    }
    pub(crate) fn issue(&mut self, id: ObjectId, limits: Limits) -> Result<()> {
        valid_id(id)?;
        if self.issued.contains(&id) {
            return Err(Error::DuplicateId(id));
        }
        if self.issued.len() >= limits.max_ids {
            return Err(Error::LimitExceeded);
        }
        self.issued.insert(id);
        Ok(())
    }
    pub fn validate(&self, limits: Limits) -> Result<()> {
        limits.validate()?;
        valid_revision(self.revision)?;
        if self.issued.len() > limits.max_ids
            || self.slides.len() > limits.max_slides
            || self.layouts.is_empty()
            || self.layouts.len() > limits.max_ids
        {
            return Err(Error::LimitExceeded);
        }
        for id in &self.issued {
            valid_id(*id)?;
        }
        let mut live = BTreeSet::new();
        let mut insert = |id| -> Result<()> {
            valid_id(id)?;
            if !self.issued.contains(&id) {
                return Err(Error::InvalidReference);
            }
            if !live.insert(id) {
                return Err(Error::DuplicateId(id));
            }
            Ok(())
        };
        insert(self.id)?;
        insert(self.theme.id)?;
        text(&self.title, limits)?;
        self.theme.slide_size.validate()?;
        if self.theme.fonts.is_empty()
            || self.theme.colors.is_empty()
            || self.theme.fonts.len() > 64
            || self.theme.colors.len() > 256
        {
            return Err(Error::InvalidTheme);
        }
        for (name, font) in &self.theme.fonts {
            text(name, limits)?;
            text(font, limits)?;
            if name.trim().is_empty() || font.trim().is_empty() {
                return Err(Error::InvalidTheme);
            }
        }
        for name in self.theme.colors.keys() {
            text(name, limits)?;
            if name.trim().is_empty() {
                return Err(Error::InvalidTheme);
            }
        }
        for layout in &self.layouts {
            insert(layout.id)?;
            text(&layout.name, limits)?;
        }
        for slide in &self.slides {
            insert(slide.id)?;
            text(&slide.notes, limits)?;
            if !self.layouts.iter().any(|l| l.id == slide.layout) {
                return Err(Error::InvalidReference);
            }
            if slide.objects.len() > limits.max_objects_per_slide {
                return Err(Error::LimitExceeded);
            }
            for object in &slide.objects {
                insert(object.id)?;
                object.geometry.validate()?;
                match &object.content {
                    Content::Text(t) | Content::Shape(t) => text(t, limits)?,
                    Content::EmbeddedReference if object.source.is_none() => {
                        return Err(Error::InvalidReference)
                    }
                    Content::EmbeddedReference => (),
                }
                if let Some(token) = &object.theme_token {
                    if !self.theme.colors.contains_key(token)
                        && !self.theme.fonts.contains_key(token)
                    {
                        return Err(Error::InvalidTheme);
                    }
                }
                if let Some(source) = &object.source {
                    valid_id(source.resource)?;
                    valid_revision(source.revision)?;
                    if let Some(id) = source.object {
                        valid_id(id)?;
                    }
                    text(&source.label, limits)?;
                    if let Some(locator) = &source.locator {
                        text(locator, limits)?;
                    }
                }
            }
        }
        let navigable: BTreeSet<_> = self
            .slides
            .iter()
            .flat_map(|s| std::iter::once(s.id).chain(s.objects.iter().map(|o| o.id)))
            .collect();
        for target in self
            .slides
            .iter()
            .flat_map(|s| &s.objects)
            .filter_map(|o| o.target)
        {
            if !navigable.contains(&target) {
                return Err(Error::InvalidReference);
            }
        }
        crate::native::encoded_len(self, limits.max_bytes).map(|_| ())
    }
}
