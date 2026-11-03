use crate::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Edit {
    AddSlide {
        index: usize,
        slide: Slide,
    },
    DeleteSlide(ObjectId),
    /// Object IDs supplied in original object order; internal copied targets remap.
    DuplicateSlide {
        slide: ObjectId,
        index: usize,
        id: ObjectId,
        object_ids: Vec<ObjectId>,
    },
    ReorderSlide {
        slide: ObjectId,
        index: usize,
    },
    AddObject {
        slide: ObjectId,
        index: usize,
        object: Object,
    },
    DeleteObject(ObjectId),
    SetObject(Object),
    SetNotes {
        slide: ObjectId,
        notes: String,
    },
    SetLayout {
        slide: ObjectId,
        layout: ObjectId,
    },
    SetTheme(Theme),
    SetTitle(String),
}
impl Presentation {
    /// Entire ordered batch commits once, or nothing (including issued IDs) changes.
    /// IDs/indices are explicit inputs, never derived from display position.
    pub fn apply(
        &mut self,
        expected: RevisionId,
        edits: &[Edit],
        limits: Limits,
    ) -> Result<RevisionId> {
        if expected != self.revision {
            return Err(Error::Conflict {
                expected,
                actual: self.revision,
            });
        }
        self.validate(limits)?;
        if edits.is_empty() || edits.len() > limits.max_operations {
            return Err(Error::LimitExceeded);
        }
        // Check untrusted input before cloning it into the candidate snapshot.
        let mut payload = 0usize;
        for edit in edits {
            payload = payload
                .checked_add(edit.payload(limits)?)
                .ok_or(Error::LimitExceeded)?;
            if payload > limits.max_bytes {
                return Err(Error::LimitExceeded);
            }
        }
        let revision = RevisionId::new(
            self.revision
                .0
                .checked_add(1)
                .ok_or(Error::InvalidRevision)?,
        )
        .map_err(|_| Error::InvalidRevision)?;
        let mut candidate = self.clone();
        let mut snapshot_bytes = native::encoded_len(&candidate, limits.max_bytes)?;
        for edit in edits {
            // Intermediate snapshots must fit even if a later edit removes the
            // content. Measure the current source (including earlier edits) before
            // either a content clone or insertion into the candidate.
            let next_bytes = candidate.prospective_bytes(edit, snapshot_bytes, limits)?;
            if let Edit::DuplicateSlide { slide, .. } = edit {
                let source = &candidate.slides[candidate.slide_index(*slide)?];
                payload = payload
                    .checked_add(native::slide_len(source, limits.max_bytes)?)
                    .ok_or(Error::LimitExceeded)?;
                if payload > limits.max_bytes {
                    return Err(Error::LimitExceeded);
                }
            }
            candidate.edit(edit, limits)?;
            snapshot_bytes = next_bytes;
        }
        candidate.revision = revision;
        candidate.validate(limits)?;
        *self = candidate;
        Ok(revision)
    }
    fn slide_index(&self, id: ObjectId) -> Result<usize> {
        self.slides
            .iter()
            .position(|s| s.id == id)
            .ok_or(Error::MissingObject(id))
    }
    fn object_index(&self, id: ObjectId) -> Result<(usize, usize)> {
        self.slides
            .iter()
            .enumerate()
            .find_map(|(s, slide)| {
                slide
                    .objects
                    .iter()
                    .position(|o| o.id == id)
                    .map(|o| (s, o))
            })
            .ok_or(Error::MissingObject(id))
    }
    /// Exact native-size changes; fixed-width collection counts do not grow.
    /// Deletions retain issued IDs, while insertions add 8-byte tombstones.
    fn prospective_bytes(&self, edit: &Edit, current: usize, limits: Limits) -> Result<usize> {
        let bound = limits.max_bytes;
        let (removed, added) = match edit {
            Edit::AddSlide { slide, .. } => (
                0,
                native::slide_len(slide, bound)?
                    .checked_add(issued_bytes(slide.objects.len())?)
                    .ok_or(Error::LimitExceeded)?,
            ),
            Edit::DuplicateSlide {
                slide, object_ids, ..
            } => {
                let source = &self.slides[self.slide_index(*slide)?];
                if object_ids.len() != source.objects.len() {
                    return Err(Error::InvalidReference);
                }
                (
                    0,
                    native::slide_len(source, bound)?
                        .checked_add(issued_bytes(source.objects.len())?)
                        .ok_or(Error::LimitExceeded)?,
                )
            }
            Edit::DeleteSlide(id) => (
                native::slide_len(&self.slides[self.slide_index(*id)?], bound)?,
                0,
            ),
            Edit::AddObject { object, .. } => (
                0,
                native::object_len(object, bound)?
                    .checked_add(8)
                    .ok_or(Error::LimitExceeded)?,
            ),
            Edit::DeleteObject(id) => {
                let (s, o) = self.object_index(*id)?;
                (native::object_len(&self.slides[s].objects[o], bound)?, 0)
            }
            Edit::SetObject(object) => {
                let (s, o) = self.object_index(object.id)?;
                (
                    native::object_len(&self.slides[s].objects[o], bound)?,
                    native::object_len(object, bound)?,
                )
            }
            Edit::SetNotes { slide, notes } => (
                native::text_len(&self.slides[self.slide_index(*slide)?].notes, bound)?,
                native::text_len(notes, bound)?,
            ),
            Edit::SetTheme(theme) => (
                native::theme_len(&self.theme, bound)?,
                native::theme_len(theme, bound)?,
            ),
            Edit::SetTitle(title) => (
                native::text_len(&self.title, bound)?,
                native::text_len(title, bound)?,
            ),
            Edit::ReorderSlide { .. } | Edit::SetLayout { .. } => (0, 0),
        };
        let next = current
            .checked_sub(removed)
            .and_then(|n| n.checked_add(added))
            .ok_or(Error::LimitExceeded)?;
        if next > bound {
            return Err(Error::LimitExceeded);
        }
        Ok(next)
    }
    fn edit(&mut self, edit: &Edit, limits: Limits) -> Result<()> {
        match edit {
            Edit::AddSlide { index, slide } => {
                if *index > self.slides.len() {
                    return Err(Error::InvalidIndex);
                }
                if self.slides.len() >= limits.max_slides
                    || slide.objects.len() > limits.max_objects_per_slide
                {
                    return Err(Error::LimitExceeded);
                }
                self.issue(slide.id, limits)?;
                for object in &slide.objects {
                    self.issue(object.id, limits)?;
                }
                self.slides.insert(*index, slide.clone());
            }
            Edit::DeleteSlide(id) => {
                let index = self.slide_index(*id)?;
                self.slides.remove(index);
            }
            Edit::DuplicateSlide {
                slide,
                index,
                id,
                object_ids,
            } => {
                let original = self.slide_index(*slide)?;
                if object_ids.len() != self.slides[original].objects.len() {
                    return Err(Error::InvalidReference);
                }
                if *index > self.slides.len() {
                    return Err(Error::InvalidIndex);
                }
                if self.slides.len() >= limits.max_slides {
                    return Err(Error::LimitExceeded);
                }
                self.issue(*id, limits)?;
                for new in object_ids {
                    self.issue(*new, limits)?;
                }
                let mut mapping = BTreeMap::from([(*slide, *id)]);
                for (object, new) in self.slides[original].objects.iter().zip(object_ids) {
                    mapping.insert(object.id, *new);
                }
                let mut copy = self.slides[original].clone();
                copy.id = *id;
                for object in &mut copy.objects {
                    object.id = mapping[&object.id];
                    object.target = object
                        .target
                        .map(|target| mapping.get(&target).copied().unwrap_or(target));
                }
                self.slides.insert(*index, copy);
            }
            Edit::ReorderSlide { slide, index } => {
                let old = self.slide_index(*slide)?;
                if *index >= self.slides.len() {
                    return Err(Error::InvalidIndex);
                }
                let slide = self.slides.remove(old);
                self.slides.insert(*index, slide);
            }
            Edit::AddObject {
                slide,
                index,
                object,
            } => {
                let s = self.slide_index(*slide)?;
                if *index > self.slides[s].objects.len() {
                    return Err(Error::InvalidIndex);
                }
                if self.slides[s].objects.len() >= limits.max_objects_per_slide {
                    return Err(Error::LimitExceeded);
                }
                self.issue(object.id, limits)?;
                self.slides[s].objects.insert(*index, object.clone());
            }
            Edit::DeleteObject(id) => {
                let (s, o) = self.object_index(*id)?;
                self.slides[s].objects.remove(o);
            }
            Edit::SetObject(object) => {
                let (s, o) = self.object_index(object.id)?;
                self.slides[s].objects[o] = object.clone();
            }
            Edit::SetNotes { slide, notes } => {
                let s = self.slide_index(*slide)?;
                self.slides[s].notes = notes.clone();
            }
            Edit::SetLayout { slide, layout } => {
                let s = self.slide_index(*slide)?;
                self.slides[s].layout = *layout;
            }
            Edit::SetTheme(theme) => {
                if theme.id != self.theme.id {
                    return Err(Error::InvalidReference);
                }
                self.theme = theme.clone();
            }
            Edit::SetTitle(title) => self.title = title.clone(),
        }
        Ok(())
    }
}

fn issued_bytes(object_count: usize) -> Result<usize> {
    object_count
        .checked_add(1)
        .and_then(|n| n.checked_mul(8))
        .ok_or(Error::LimitExceeded)
}

impl Edit {
    fn payload(&self, limits: Limits) -> Result<usize> {
        let mut bytes = 0usize;
        let mut add = |s: &str| -> Result<()> {
            crate::model::text(s, limits)?;
            bytes = bytes.checked_add(s.len()).ok_or(Error::LimitExceeded)?;
            Ok(())
        };
        match self {
            Self::AddSlide { slide, .. } => {
                if slide.objects.len() > limits.max_objects_per_slide {
                    return Err(Error::LimitExceeded);
                }
                add(&slide.notes)?;
                for object in &slide.objects {
                    object_payload(object, &mut add)?;
                }
            }
            Self::AddObject { object, .. } | Self::SetObject(object) => {
                object_payload(object, &mut add)?
            }
            Self::SetNotes { notes, .. } | Self::SetTitle(notes) => add(notes)?,
            Self::SetTheme(theme) => {
                if theme.fonts.len() > 64 || theme.colors.len() > 256 {
                    return Err(Error::LimitExceeded);
                }
                for (key, value) in &theme.fonts {
                    add(key)?;
                    add(value)?;
                }
                for key in theme.colors.keys() {
                    add(key)?;
                }
            }
            Self::DuplicateSlide { object_ids, .. }
                if object_ids.len() > limits.max_objects_per_slide =>
            {
                return Err(Error::LimitExceeded);
            }
            _ => (),
        }
        Ok(bytes)
    }
}
fn object_payload(object: &Object, add: &mut impl FnMut(&str) -> Result<()>) -> Result<()> {
    object.geometry.validate()?;
    match &object.content {
        Content::Text(t) | Content::Shape(t) => add(t)?,
        _ => (),
    }
    if let Some(token) = &object.theme_token {
        add(token)?;
    }
    if let Some(source) = &object.source {
        add(&source.label)?;
        if let Some(locator) = &source.locator {
            add(locator)?;
        }
    }
    Ok(())
}
