use std::collections::BTreeSet;

use crate::{
    localization::{self, Locale},
    CapabilityAuthorizer, EntryKind, FileEntry, FilesContextSnapshot, FilesError, FilesErrorKind,
    FilesService, FilesystemProvider, Location, ResourceId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewLoadState {
    Empty,
    Loading,
    Ready,
    Error(FilesErrorKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectorTab {
    Preview,
    Metadata,
    Activity,
    Versions,
    Provenance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidebarItem {
    pub label_key: &'static str,
    pub destination: Option<Location>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocationItem {
    pub label: String,
    pub location: Location,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesLayout {
    pub sidebar: Vec<SidebarItem>,
    pub breadcrumbs: Vec<LocationItem>,
    pub resources: Vec<FileEntry>,
    pub selected: BTreeSet<ResourceId>,
    pub inspector: Option<FileEntry>,
    pub inspector_tab: InspectorTab,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesViewState {
    pub location: Location,
    pub breadcrumbs: Vec<LocationItem>,
    pub entries: Vec<FileEntry>,
    pub selected: BTreeSet<ResourceId>,
    pub focused: Option<ResourceId>,
    pub load_state: ViewLoadState,
    pub inspector_visible: bool,
    pub inspector_tab: InspectorTab,
    pub search_query: String,
}

impl FilesViewState {
    pub fn new(location: Location) -> Self {
        Self {
            breadcrumbs: breadcrumbs(&location),
            location,
            entries: Vec::new(),
            selected: BTreeSet::new(),
            focused: None,
            load_state: ViewLoadState::Empty,
            inspector_visible: true,
            inspector_tab: InspectorTab::Metadata,
            search_query: String::new(),
        }
    }

    pub fn layout(&self) -> FilesLayout {
        let inspector = self
            .focused
            .and_then(|id| self.entries.iter().find(|entry| entry.id == id))
            .cloned();
        FilesLayout {
            sidebar: default_sidebar(),
            breadcrumbs: self.breadcrumbs.clone(),
            resources: self.entries.clone(),
            selected: self.selected.clone(),
            inspector: self.inspector_visible.then_some(inspector).flatten(),
            inspector_tab: self.inspector_tab,
        }
    }

    pub fn selection_context(&self) -> Vec<FileEntry> {
        self.entries
            .iter()
            .filter(|entry| self.selected.contains(&entry.id))
            .cloned()
            .collect()
    }

    pub fn select(&mut self, id: ResourceId, additive: bool) -> Result<(), FilesError> {
        if !self.entries.iter().any(|entry| entry.id == id) {
            return Err(FilesError::new(FilesErrorKind::NotFound));
        }
        if !additive {
            self.selected.clear();
        }
        self.selected.insert(id);
        self.focused = Some(id);
        Ok(())
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.focused = None;
    }

    pub fn navigate_to(&mut self, location: Location) {
        self.location = location;
        self.breadcrumbs = breadcrumbs(&self.location);
        self.entries.clear();
        self.clear_selection();
        self.load_state = ViewLoadState::Empty;
    }

    pub fn parent_location(&self) -> Option<Location> {
        self.location.parent()
    }
}

pub struct FilesApp {
    pub state: FilesViewState,
}

pub trait ContextPublisher {
    fn publish(&mut self, context: &FilesContextSnapshot) -> Result<(), String>;
}

impl FilesApp {
    pub fn new(initial_location: Location) -> Self {
        Self {
            state: FilesViewState::new(initial_location),
        }
    }

    pub fn context_snapshot(&self) -> FilesContextSnapshot {
        FilesContextSnapshot {
            current_location: self.state.location.clone(),
            selected_resources: self.state.selected.iter().copied().collect(),
            focused_resource: self.state.focused,
            active_workspace: None,
        }
    }

    pub fn publish_context(&self, publisher: &mut impl ContextPublisher) -> Result<(), String> {
        publisher.publish(&self.context_snapshot())
    }

    pub fn refresh<P, A>(&mut self, service: &FilesService<P, A>) -> Result<(), FilesError>
    where
        P: FilesystemProvider,
        A: CapabilityAuthorizer,
    {
        self.state.load_state = ViewLoadState::Loading;
        match service.list(&self.state.location) {
            Ok(entries) => {
                self.state.entries = entries;
                self.state
                    .selected
                    .retain(|id| self.state.entries.iter().any(|entry| entry.id == *id));
                if !self
                    .state
                    .focused
                    .is_some_and(|id| self.state.selected.contains(&id))
                {
                    self.state.focused = self.state.selected.iter().next().copied();
                }
                self.state.load_state = ViewLoadState::Ready;
                Ok(())
            }
            Err(error) => {
                self.state.entries.clear();
                self.state.clear_selection();
                self.state.load_state = ViewLoadState::Error(error.kind);
                Err(error)
            }
        }
    }

    pub fn navigate<P, A>(
        &mut self,
        service: &FilesService<P, A>,
        location: Location,
    ) -> Result<(), FilesError>
    where
        P: FilesystemProvider,
        A: CapabilityAuthorizer,
    {
        self.state.navigate_to(location);
        self.refresh(service)
    }

    pub fn navigate_parent<P, A>(
        &mut self,
        service: &FilesService<P, A>,
    ) -> Result<bool, FilesError>
    where
        P: FilesystemProvider,
        A: CapabilityAuthorizer,
    {
        let Some(parent) = self.state.parent_location() else {
            return Ok(false);
        };
        self.navigate(service, parent)?;
        Ok(true)
    }
}

pub fn render_three_pane(layout: &FilesLayout, empty_message: &str, locale: Locale) -> String {
    let sidebar_width = 18;
    let resource_width = 34;
    let mut output = String::new();
    output.push_str(&format!(
        "{:<sidebar_width$} | {:<resource_width$} | {} ({:?})\n",
        "Locations", "Resources", "Inspector", layout.inspector_tab
    ));
    output.push_str(&format!(
        "{:-<sidebar_width$}-+-{:-<resource_width$}-+-{:-<24}\n",
        "", "", ""
    ));

    let sidebar = layout
        .sidebar
        .iter()
        .map(|item| {
            localization::text(locale, item.label_key)
                .unwrap_or(item.label_key)
                .to_owned()
        })
        .collect::<Vec<_>>();
    let resources = if layout.resources.is_empty() {
        vec![empty_message.to_owned()]
    } else {
        layout
            .resources
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let marker = match entry.kind {
                    EntryKind::Folder => "[D]",
                    EntryKind::File => "[F]",
                    EntryKind::Symlink => "[L]",
                    EntryKind::Unsupported => "[?]",
                };
                let selected = if layout.selected.contains(&entry.id) {
                    "*"
                } else {
                    " "
                };
                format!("{:>3} {selected}{marker} {}", index + 1, entry.name)
            })
            .collect()
    };
    let inspector = layout.inspector.as_ref().map_or_else(
        || vec!["No selection".to_owned()],
        |entry| {
            vec![
                entry.name.to_string(),
                format!("Kind: {:?}", entry.kind),
                format!(
                    "Size: {}",
                    entry
                        .size_bytes
                        .map_or("—".to_owned(), |n| format!("{n} B"))
                ),
                format!("Location: {}", entry.location),
            ]
        },
    );

    for index in 0..sidebar.len().max(resources.len()).max(inspector.len()) {
        let left = sidebar.get(index).map_or("", String::as_str);
        let middle = resources.get(index).map_or("", String::as_str);
        let right = inspector.get(index).map_or("", String::as_str);
        output.push_str(&format!(
            "{left:<sidebar_width$} | {middle:<resource_width$} | {right}\n"
        ));
    }
    output
}

fn breadcrumbs(location: &Location) -> Vec<LocationItem> {
    let mut result = vec![LocationItem {
        label: "Home".to_owned(),
        location: Location::root(),
    }];
    let mut current = Location::root();
    for part in location.components() {
        current =
            current.join(&crate::FileName::parse(part).expect("validated location component"));
        result.push(LocationItem {
            label: part.to_owned(),
            location: current.clone(),
        });
    }
    result
}

fn default_sidebar() -> Vec<SidebarItem> {
    [
        ("files.sidebar.home", Some(Location::root())),
        ("files.sidebar.recent", None),
        ("files.sidebar.starred", None),
        ("files.sidebar.workspaces", None),
        ("files.sidebar.downloads", None),
        ("files.sidebar.documents", None),
        ("files.sidebar.locations", None),
        ("files.sidebar.devices", None),
        ("files.sidebar.trash", None),
    ]
    .into_iter()
    .map(|(label_key, destination)| SidebarItem {
        label_key,
        destination,
    })
    .collect()
}
