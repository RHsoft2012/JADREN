//! Platform-neutral retained layout model used by Jadren UI backends.
//!
//! The current Win32 preview has a C renderer with the same bounded layout
//! semantics. This crate is deliberately free of window-system handles: it
//! provides the deterministic tree, scope validation, geometry snapshot and
//! bounded pointer-event state machine that a native backend can consume.

/// Maximum number of container nodes in one retained tree.
pub const MAX_LAYOUT_NODES: usize = 16;
/// Maximum number of children attached to one container.
pub const MAX_LAYOUT_CHILDREN: usize = 24;
/// Highest source-level node id accepted by the retained contract.
pub const MAX_NODE_ID: u8 = 127;
/// Maximum number of input events retained between backend polls.
pub const MAX_UI_EVENTS: usize = 32;
/// Maximum number of commands in one headless retained frame.
pub const MAX_RENDER_COMMANDS: usize = MAX_NODE_ID as usize + 3;

const ROOT_MARGIN: i32 = 16;
const ROOT_PADDING: i32 = 16;
const ROOT_GAP: i32 = 12;
const MIN_WINDOW_WIDTH: i32 = 360;
const MAX_WINDOW_WIDTH: i32 = 1600;
const MIN_WINDOW_HEIGHT: i32 = 240;
const MAX_WINDOW_HEIGHT: i32 = 1000;

/// Stable source-level identity for a retained node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(u8);

impl NodeId {
    /// Converts a Jadren `Int32` node id into the bounded model identity.
    #[must_use]
    pub const fn new(raw: i32) -> Option<Self> {
        if raw > 0 && raw <= MAX_NODE_ID as i32 {
            Some(Self(raw as u8))
        } else {
            None
        }
    }

    /// Returns the integer value exposed to Jadren source.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Orientation of a retained container.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Orientation {
    /// Children are stacked from top to bottom.
    Column,
    /// Children are placed from left to right.
    Row,
}

/// Cross-axis alignment used by the retained layout contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Align {
    /// Place children at the start edge.
    Start,
    /// Centre children or a row group.
    Center,
    /// Place children at the end edge.
    End,
}

impl Align {
    /// Maps the source integer convention `0=start`, `1=center`, `2=end`.
    #[must_use]
    pub const fn from_raw(raw: i32) -> Self {
        match raw {
            1 => Self::Center,
            2 => Self::End,
            _ => Self::Start,
        }
    }
}

/// Geometry and cross-axis policy for a retained container.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContainerSpec {
    /// Requested width in pixels.
    pub width: i32,
    /// Requested height in pixels.
    pub height: i32,
    /// Inner padding in pixels.
    pub padding: i32,
    /// Gap between children in pixels.
    pub gap: i32,
    /// Cross-axis alignment.
    pub align: Align,
    /// Whether the parent may stretch this container on its cross axis.
    pub stretch: bool,
}

impl ContainerSpec {
    /// Creates a container specification from source-level layout values.
    #[must_use]
    pub const fn new(
        width: i32,
        height: i32,
        padding: i32,
        gap: i32,
        align: Align,
        stretch: bool,
    ) -> Self {
        Self {
            width,
            height,
            padding,
            gap,
            align,
            stretch,
        }
    }

    fn normalized(self) -> Self {
        Self {
            width: clamp_dimension(self.width, 1, 4000),
            height: clamp_dimension(self.height, 1, 4000),
            padding: clamp_dimension(self.padding, 0, 128),
            gap: clamp_dimension(self.gap, 0, 128),
            ..self
        }
    }
}

/// Widget category preserved in a headless snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetKind {
    /// Root retained container created by `ui_app_begin`.
    Root,
    /// Vertical retained panel created by `ui_app_panel`.
    Panel,
    /// Horizontal retained container for backend composition tests.
    Row,
    /// Horizontal retained top bar with backend-owned background styling.
    TopBar,
    /// Retained popup-menu trigger whose options are backend-owned.
    Menu,
    /// Static retained label.
    Label,
    /// Event retained button.
    Button,
    /// Retained single-line text input.
    TextInput,
    /// Retained checkbox with an explicit callback/event id.
    Checkbox,
    /// Retained single-selection dropdown with an explicit callback/event id.
    Select,
    /// Retained bounded dynamic list with an explicit callback/event id.
    List,
    /// Retained bounded report table with an explicit callback/event id.
    Table,
}

impl WidgetKind {
    const fn is_interactive(self) -> bool {
        matches!(
            self,
            Self::Button
                | Self::Menu
                | Self::TextInput
                | Self::Checkbox
                | Self::Select
                | Self::List
                | Self::Table
        )
    }
}

/// Integer rectangle emitted by the layout pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    /// Left coordinate in the backend's client space.
    pub x: i32,
    /// Top coordinate in the backend's client space.
    pub y: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
}

impl Rect {
    const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

/// One placed retained widget in a headless snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Placement {
    /// Source-level node identity.
    pub id: NodeId,
    /// Widget category.
    pub kind: WidgetKind,
    /// Explicit Jadren callback/event id associated with this widget.
    pub event_id: i32,
    /// Calculated rectangle.
    pub rect: Rect,
}

/// Deterministic result of one layout pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutSnapshot {
    placements: [Option<Placement>; MAX_NODE_ID as usize + 1],
}

impl LayoutSnapshot {
    /// Returns the calculated rectangle and kind for one node id.
    #[must_use]
    pub fn placement(&self, id: NodeId) -> Option<Placement> {
        self.placements[id.index()]
    }

    /// Iterates placements in ascending source node-id order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = Placement> + '_ {
        self.placements.iter().flatten().copied()
    }

    /// Returns the topmost interactive widget under a client-space pointer coordinate.
    ///
    /// Retained children are visited in reverse source order so a backend can
    /// use the same rule when two future widgets overlap. Coordinates use the
    /// usual half-open rectangle convention: left/top are included and
    /// right/bottom are excluded.
    #[must_use]
    pub fn hit_test(&self, x: i32, y: i32) -> Option<Placement> {
        self.iter()
            .rev()
            .find(|placement| placement.kind.is_interactive() && placement.rect.contains(x, y))
    }

    fn set(&mut self, placement: Placement) {
        self.placements[placement.id.index()] = Some(placement);
    }

    /// Builds a deterministic backend-neutral frame for the given viewport.
    #[must_use]
    pub fn render_frame(&self, viewport_width: i32, viewport_height: i32) -> HeadlessFrame {
        HeadlessFrame::from_snapshot(self, viewport_width, viewport_height)
    }
}

/// One command consumed by a retained backend renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderCommand {
    /// Starts a frame in the normalized client viewport.
    BeginFrame { width: i32, height: i32 },
    /// Places one retained widget in source node-id order.
    Widget(Placement),
    /// Marks the end of the frame command stream.
    EndFrame,
}

/// Deterministic, allocation-free render command stream for a retained tree.
///
/// A native backend may translate these commands to Win32, X11, a test
/// renderer or another window system without reimplementing layout ordering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadlessFrame {
    commands: [Option<RenderCommand>; MAX_RENDER_COMMANDS],
    command_count: usize,
}

impl HeadlessFrame {
    fn from_snapshot(snapshot: &LayoutSnapshot, viewport_width: i32, viewport_height: i32) -> Self {
        let (width, height) = normalize_viewport(viewport_width, viewport_height);
        let mut frame = Self {
            commands: [None; MAX_RENDER_COMMANDS],
            command_count: 0,
        };
        frame.push(RenderCommand::BeginFrame { width, height });
        for placement in snapshot.iter() {
            frame.push(RenderCommand::Widget(placement));
        }
        frame.push(RenderCommand::EndFrame);
        frame
    }

    fn push(&mut self, command: RenderCommand) {
        debug_assert!(self.command_count < MAX_RENDER_COMMANDS);
        self.commands[self.command_count] = Some(command);
        self.command_count += 1;
    }

    /// Returns the number of commands in this frame.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.command_count
    }

    /// Returns whether the frame contains no commands.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.command_count == 0
    }

    /// Iterates commands in renderer order.
    pub fn iter(&self) -> impl Iterator<Item = RenderCommand> + '_ {
        self.commands[..self.command_count]
            .iter()
            .flatten()
            .copied()
    }
}

/// Pointer/interaction transition emitted by the retained event model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiEventKind {
    /// The pointer entered the target button.
    HoverEnter,
    /// The pointer left the target button.
    HoverLeave,
    /// The primary pointer button was pressed on the target.
    Pressed,
    /// The primary pointer button was released after a press.
    Released,
    /// A press and release completed on the same target.
    Clicked,
    /// A press was released outside its original target.
    Cancelled,
}

/// One backend-neutral event delivered to a Jadren callback boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiEvent {
    /// Stable retained node identity.
    pub target: NodeId,
    /// Explicit source callback/event id associated with the node.
    pub event_id: i32,
    /// Transition kind.
    pub kind: UiEventKind,
}

/// Errors returned when a backend cannot enqueue another retained event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionError {
    /// The bounded event queue is full.
    EventCapacity,
    /// A retained node disappeared between layout snapshots.
    TargetMissing,
}

/// Bounded pointer state machine shared by native UI backends.
///
/// The model owns no platform handles and never allocates. A backend feeds it
/// client-space pointer transitions against a `LayoutSnapshot`, then polls
/// the deterministic event queue and maps `event_id` to the source callback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractionModel {
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
    focused: Option<NodeId>,
    events: [Option<UiEvent>; MAX_UI_EVENTS],
    event_count: usize,
}

impl InteractionModel {
    /// Creates an idle interaction state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            hovered: None,
            pressed: None,
            focused: None,
            events: [None; MAX_UI_EVENTS],
            event_count: 0,
        }
    }

    /// Returns the currently hovered node, if any.
    #[must_use]
    pub const fn hovered(&self) -> Option<NodeId> {
        self.hovered
    }

    /// Returns the node holding the active pointer press, if any.
    #[must_use]
    pub const fn pressed(&self) -> Option<NodeId> {
        self.pressed
    }

    /// Returns the node that received the last successful pointer press.
    #[must_use]
    pub const fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Iterates queued events without removing them.
    pub fn events(&self) -> impl Iterator<Item = UiEvent> + '_ {
        self.events[..self.event_count].iter().flatten().copied()
    }

    /// Removes all queued events while preserving hover/press state.
    pub fn clear_events(&mut self) {
        self.events = [None; MAX_UI_EVENTS];
        self.event_count = 0;
    }

    /// Removes and returns the oldest queued event.
    pub fn poll_event(&mut self) -> Option<UiEvent> {
        let event = self.events[0]?;
        for index in 1..self.event_count {
            self.events[index - 1] = self.events[index];
        }
        self.event_count -= 1;
        self.events[self.event_count] = None;
        Some(event)
    }

    /// Feeds a pointer move and updates hover enter/leave transitions.
    pub fn pointer_move(
        &mut self,
        snapshot: &LayoutSnapshot,
        x: i32,
        y: i32,
    ) -> Result<(), InteractionError> {
        self.sync_hover(
            snapshot,
            snapshot.hit_test(x, y).map(|placement| placement.id),
        )
    }

    /// Feeds a primary pointer-down transition.
    pub fn pointer_down(
        &mut self,
        snapshot: &LayoutSnapshot,
        x: i32,
        y: i32,
    ) -> Result<(), InteractionError> {
        let target = snapshot.hit_test(x, y).map(|placement| placement.id);
        let required = self.hover_transition_count(target) + usize::from(target.is_some());
        self.ensure_capacity(required)?;
        self.sync_hover(snapshot, target)?;
        let Some(target) = target else {
            self.pressed = None;
            self.focused = None;
            return Ok(());
        };
        self.pressed = Some(target);
        self.focused = Some(target);
        self.push_event(snapshot, target, UiEventKind::Pressed)
    }

    /// Feeds a primary pointer-up transition and emits click only on-target.
    pub fn pointer_up(
        &mut self,
        snapshot: &LayoutSnapshot,
        x: i32,
        y: i32,
    ) -> Result<(), InteractionError> {
        let target = snapshot.hit_test(x, y).map(|placement| placement.id);
        let required =
            self.hover_transition_count(target) + usize::from(self.pressed.is_some()) * 2;
        self.ensure_capacity(required)?;
        self.sync_hover(snapshot, target)?;
        let Some(pressed) = self.pressed else {
            return Ok(());
        };
        if snapshot.placement(pressed).is_none() {
            return Err(InteractionError::TargetMissing);
        }
        self.pressed = None;
        self.push_event(snapshot, pressed, UiEventKind::Released)?;
        if target == Some(pressed) {
            self.push_event(snapshot, pressed, UiEventKind::Clicked)?;
        } else {
            self.push_event(snapshot, pressed, UiEventKind::Cancelled)?;
        }
        Ok(())
    }

    /// Feeds a native mouse-leave transition.
    ///
    /// A held press is intentionally preserved until pointer-up, matching
    /// native pointer capture; releasing outside then produces `Cancelled`.
    pub fn pointer_leave(&mut self, snapshot: &LayoutSnapshot) -> Result<(), InteractionError> {
        self.sync_hover(snapshot, None)
    }

    /// Cancels a held pointer press, for example after native capture loss.
    pub fn pointer_cancel(&mut self, snapshot: &LayoutSnapshot) -> Result<(), InteractionError> {
        let Some(pressed) = self.pressed else {
            return Ok(());
        };
        self.ensure_capacity(1)?;
        if snapshot.placement(pressed).is_none() {
            return Err(InteractionError::TargetMissing);
        }
        self.pressed = None;
        self.push_event(snapshot, pressed, UiEventKind::Cancelled)
    }

    fn hover_transition_count(&self, target: Option<NodeId>) -> usize {
        if self.hovered == target {
            0
        } else {
            usize::from(self.hovered.is_some()) + usize::from(target.is_some())
        }
    }

    fn sync_hover(
        &mut self,
        snapshot: &LayoutSnapshot,
        target: Option<NodeId>,
    ) -> Result<(), InteractionError> {
        if self.hovered == target {
            return Ok(());
        }
        let required = self.hover_transition_count(target);
        self.ensure_capacity(required)?;
        if let Some(previous) = self.hovered {
            self.push_event(snapshot, previous, UiEventKind::HoverLeave)?;
        }
        if let Some(current) = target {
            self.push_event(snapshot, current, UiEventKind::HoverEnter)?;
        }
        self.hovered = target;
        Ok(())
    }

    fn ensure_capacity(&self, additional: usize) -> Result<(), InteractionError> {
        if self.event_count.saturating_add(additional) > MAX_UI_EVENTS {
            Err(InteractionError::EventCapacity)
        } else {
            Ok(())
        }
    }

    fn push_event(
        &mut self,
        snapshot: &LayoutSnapshot,
        target: NodeId,
        kind: UiEventKind,
    ) -> Result<(), InteractionError> {
        let placement = snapshot
            .placement(target)
            .ok_or(InteractionError::TargetMissing)?;
        if self.event_count >= MAX_UI_EVENTS {
            return Err(InteractionError::EventCapacity);
        }
        self.events[self.event_count] = Some(UiEvent {
            target,
            event_id: placement.event_id,
            kind,
        });
        self.event_count += 1;
        Ok(())
    }
}

impl Default for InteractionModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Platform-neutral retained backend used by tests and future native ports.
///
/// The backend owns the current layout snapshot and feeds pointer transitions
/// into the shared interaction state machine. It deliberately does not create
/// a window or allocate a render surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedHeadlessBackend {
    snapshot: LayoutSnapshot,
    interaction: InteractionModel,
}

impl RetainedHeadlessBackend {
    /// Creates a backend over one completed layout snapshot.
    #[must_use]
    pub const fn new(snapshot: LayoutSnapshot) -> Self {
        Self {
            snapshot,
            interaction: InteractionModel::new(),
        }
    }

    /// Returns the snapshot currently consumed by the backend.
    #[must_use]
    pub const fn snapshot(&self) -> &LayoutSnapshot {
        &self.snapshot
    }

    /// Replaces the layout snapshot and resets pointer/event state.
    pub fn set_snapshot(&mut self, snapshot: LayoutSnapshot) {
        self.snapshot = snapshot;
        self.interaction = InteractionModel::new();
    }

    /// Builds the deterministic render command stream for the current layout.
    #[must_use]
    pub fn frame(&self, viewport_width: i32, viewport_height: i32) -> HeadlessFrame {
        self.snapshot.render_frame(viewport_width, viewport_height)
    }

    /// Feeds a pointer move to the shared event model.
    pub fn pointer_move(&mut self, x: i32, y: i32) -> Result<(), InteractionError> {
        self.interaction.pointer_move(&self.snapshot, x, y)
    }

    /// Feeds a primary pointer-down to the shared event model.
    pub fn pointer_down(&mut self, x: i32, y: i32) -> Result<(), InteractionError> {
        self.interaction.pointer_down(&self.snapshot, x, y)
    }

    /// Feeds a primary pointer-up to the shared event model.
    pub fn pointer_up(&mut self, x: i32, y: i32) -> Result<(), InteractionError> {
        self.interaction.pointer_up(&self.snapshot, x, y)
    }

    /// Feeds a native pointer-leave transition.
    pub fn pointer_leave(&mut self) -> Result<(), InteractionError> {
        self.interaction.pointer_leave(&self.snapshot)
    }

    /// Cancels a held press, for example after native capture loss.
    pub fn pointer_cancel(&mut self) -> Result<(), InteractionError> {
        self.interaction.pointer_cancel(&self.snapshot)
    }

    /// Removes and returns the oldest backend-neutral event.
    pub fn poll_event(&mut self) -> Option<UiEvent> {
        self.interaction.poll_event()
    }
}

/// Errors returned before a backend consumes a retained layout tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// A second root was requested.
    RootAlreadyOpen,
    /// A child was added outside the current LIFO scope.
    InvalidParent,
    /// The bounded container capacity was exhausted.
    NodeCapacity,
    /// One container already has the maximum number of children.
    ChildCapacity,
    /// The source-level node-id space was exhausted.
    NodeIdCapacity,
    /// A container scope was opened beyond the bounded stack depth.
    ScopeCapacity,
    /// `end` received a node other than the current scope.
    ScopeOrder,
    /// Layout was requested before a root existed.
    MissingRoot,
    /// Layout was requested while a scope was still open.
    UnclosedScopes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChildSpec {
    id: NodeId,
    kind: WidgetKind,
    event_id: i32,
    width: i32,
    height: i32,
    stretch: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeSpec {
    id: NodeId,
    kind: WidgetKind,
    base_rect: Rect,
    orientation: Orientation,
    padding: i32,
    gap: i32,
    align: Align,
    stretch: bool,
    child_count: usize,
    children: [Option<ChildSpec>; MAX_LAYOUT_CHILDREN],
}

impl NodeSpec {
    const fn new(
        id: NodeId,
        kind: WidgetKind,
        base_rect: Rect,
        orientation: Orientation,
        spec: ContainerSpec,
    ) -> Self {
        Self {
            id,
            kind,
            base_rect,
            orientation,
            padding: spec.padding,
            gap: spec.gap,
            align: spec.align,
            stretch: spec.stretch,
            child_count: 0,
            children: [None; MAX_LAYOUT_CHILDREN],
        }
    }
}

/// Bounded retained UI tree with deterministic layout semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutTree {
    base_viewport_width: i32,
    base_viewport_height: i32,
    next_id: u16,
    node_count: usize,
    root: Option<NodeId>,
    nodes: [Option<NodeSpec>; MAX_NODE_ID as usize + 1],
    stack: [NodeId; MAX_LAYOUT_NODES],
    stack_depth: usize,
}

impl LayoutTree {
    /// Creates an empty tree for a window-sized viewport.
    #[must_use]
    pub fn new(viewport_width: i32, viewport_height: i32) -> Self {
        let (width, height) = normalize_viewport(viewport_width, viewport_height);
        Self {
            base_viewport_width: width,
            base_viewport_height: height,
            next_id: 1,
            node_count: 0,
            root: None,
            nodes: [None; MAX_NODE_ID as usize + 1],
            stack: [NodeId(0); MAX_LAYOUT_NODES],
            stack_depth: 0,
        }
    }

    /// Opens the implicit root column used by `ui_app_begin`.
    pub fn begin_root(&mut self) -> Result<NodeId, LayoutError> {
        if self.root.is_some() {
            return Err(LayoutError::RootAlreadyOpen);
        }
        let width = clamp_dimension(self.base_viewport_width - ROOT_MARGIN * 2, 1, 4000);
        let height = clamp_dimension(self.base_viewport_height - ROOT_MARGIN * 2, 1, 4000);
        let root_id = self.allocate_id()?;
        let root = self.insert_container(NodeSpec::new(
            root_id,
            WidgetKind::Root,
            Rect::new(ROOT_MARGIN, ROOT_MARGIN, width, height),
            Orientation::Column,
            ContainerSpec::new(width, height, ROOT_PADDING, ROOT_GAP, Align::Start, true),
        ))?;
        self.root = Some(root);
        self.push_scope(root)?;
        Ok(root)
    }

    /// Opens a vertical panel under the current parent scope.
    pub fn panel(&mut self, parent: NodeId, spec: ContainerSpec) -> Result<NodeId, LayoutError> {
        self.open_container(parent, WidgetKind::Panel, Orientation::Column, spec)
    }

    /// Opens a horizontal row under the current parent scope.
    pub fn row(&mut self, parent: NodeId, spec: ContainerSpec) -> Result<NodeId, LayoutError> {
        self.open_container(parent, WidgetKind::Row, Orientation::Row, spec)
    }

    /// Opens a horizontal top-bar scope under the current parent.
    pub fn top_bar(&mut self, parent: NodeId, spec: ContainerSpec) -> Result<NodeId, LayoutError> {
        self.open_container(parent, WidgetKind::TopBar, Orientation::Row, spec)
    }

    /// Adds a retained popup-menu trigger to the current parent.
    ///
    /// Menu items are declared through the platform menu contract and are not
    /// laid out as ordinary children; the trigger itself participates in the
    /// same deterministic geometry and hit-test pass as a button.
    pub fn menu(
        &mut self,
        parent: NodeId,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.insert_leaf(parent, WidgetKind::Menu, 0, width, height, stretch)
    }

    /// Adds a static label to the current parent and returns its node id.
    pub fn label(
        &mut self,
        parent: NodeId,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.insert_leaf(parent, WidgetKind::Label, 0, width, height, stretch)
    }

    /// Adds an event button to the current parent and returns its node id.
    pub fn button(
        &mut self,
        parent: NodeId,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.button_with_event(parent, 0, width, height, stretch)
    }

    /// Adds an event button with its explicit Jadren callback/event id.
    pub fn button_with_event(
        &mut self,
        parent: NodeId,
        event_id: i32,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.insert_leaf(parent, WidgetKind::Button, event_id, width, height, stretch)
    }

    /// Adds a retained single-line text input with an explicit callback/event id.
    pub fn text_input_with_event(
        &mut self,
        parent: NodeId,
        event_id: i32,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.insert_leaf(
            parent,
            WidgetKind::TextInput,
            event_id,
            width,
            height,
            stretch,
        )
    }

    /// Adds a retained checkbox with an explicit callback/event id.
    pub fn checkbox_with_event(
        &mut self,
        parent: NodeId,
        event_id: i32,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.insert_leaf(
            parent,
            WidgetKind::Checkbox,
            event_id,
            width,
            height,
            stretch,
        )
    }

    /// Adds a retained single-selection dropdown with an explicit event id.
    pub fn select_with_event(
        &mut self,
        parent: NodeId,
        event_id: i32,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.insert_leaf(parent, WidgetKind::Select, event_id, width, height, stretch)
    }

    /// Adds a retained bounded dynamic list with an explicit event id.
    pub fn list_with_event(
        &mut self,
        parent: NodeId,
        event_id: i32,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.insert_leaf(parent, WidgetKind::List, event_id, width, height, stretch)
    }

    /// Adds a retained bounded report table with an explicit event id.
    pub fn table_with_event(
        &mut self,
        parent: NodeId,
        event_id: i32,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.insert_leaf(parent, WidgetKind::Table, event_id, width, height, stretch)
    }

    /// Closes the current root or container scope in LIFO order.
    pub fn end(&mut self, node: NodeId) -> Result<(), LayoutError> {
        if self.stack_depth == 0 || self.stack[self.stack_depth - 1] != node {
            return Err(LayoutError::ScopeOrder);
        }
        self.stack_depth -= 1;
        Ok(())
    }

    /// Checks that a retained tree is complete before handing it to a backend.
    pub fn run_ready(&self) -> Result<(), LayoutError> {
        if self.root.is_none() {
            return Err(LayoutError::MissingRoot);
        }
        if self.stack_depth != 0 {
            return Err(LayoutError::UnclosedScopes);
        }
        Ok(())
    }

    /// Calculates all node rectangles for a viewport resize.
    pub fn layout(
        &self,
        viewport_width: i32,
        viewport_height: i32,
    ) -> Result<LayoutSnapshot, LayoutError> {
        self.run_ready()?;
        let root = self.root.ok_or(LayoutError::MissingRoot)?;
        let (width, _height) = normalize_viewport(viewport_width, viewport_height);
        let root_spec = self.nodes[root.index()].ok_or(LayoutError::MissingRoot)?;
        let width_delta = width - self.base_viewport_width;
        let root_width = if root_spec.stretch {
            clamp_dimension(root_spec.base_rect.width + width_delta, 1, 4000)
        } else {
            root_spec.base_rect.width
        };
        let root_rect = Rect::new(
            root_spec.base_rect.x,
            root_spec.base_rect.y,
            root_width,
            root_spec.base_rect.height,
        );
        let mut snapshot = LayoutSnapshot {
            placements: [None; MAX_NODE_ID as usize + 1],
        };
        self.layout_node(root, root_rect, &mut snapshot);
        Ok(snapshot)
    }

    /// Returns the root node id once `begin_root` has been called.
    #[must_use]
    pub const fn root(&self) -> Option<NodeId> {
        self.root
    }

    fn open_container(
        &mut self,
        parent: NodeId,
        kind: WidgetKind,
        orientation: Orientation,
        spec: ContainerSpec,
    ) -> Result<NodeId, LayoutError> {
        self.ensure_parent(parent)?;
        self.ensure_child_capacity(parent)?;
        if self.stack_depth >= MAX_LAYOUT_NODES {
            return Err(LayoutError::ScopeCapacity);
        }
        let id = self.allocate_id()?;
        let spec = spec.normalized();
        let node = NodeSpec::new(
            id,
            kind,
            Rect::new(0, 0, spec.width, spec.height),
            orientation,
            spec,
        );
        let inserted = self.insert_container(node)?;
        self.push_child(
            parent,
            ChildSpec {
                id: inserted,
                kind,
                event_id: 0,
                width: node.base_rect.width,
                height: node.base_rect.height,
                stretch: spec.stretch,
            },
        )?;
        self.push_scope(inserted)?;
        Ok(inserted)
    }

    fn insert_leaf(
        &mut self,
        parent: NodeId,
        kind: WidgetKind,
        event_id: i32,
        width: i32,
        height: i32,
        stretch: bool,
    ) -> Result<NodeId, LayoutError> {
        self.ensure_parent(parent)?;
        self.ensure_child_capacity(parent)?;
        let id = self.allocate_id()?;
        self.push_child(
            parent,
            ChildSpec {
                id,
                kind,
                event_id,
                width: clamp_dimension(width, 1, 4000),
                height: clamp_dimension(height, 1, 4000),
                stretch,
            },
        )?;
        Ok(id)
    }

    fn insert_container(&mut self, node: NodeSpec) -> Result<NodeId, LayoutError> {
        if self.node_count >= MAX_LAYOUT_NODES {
            return Err(LayoutError::NodeCapacity);
        }
        let id = node.id;
        self.nodes[id.index()] = Some(node);
        self.node_count += 1;
        Ok(id)
    }

    fn allocate_id(&mut self) -> Result<NodeId, LayoutError> {
        if self.next_id > MAX_NODE_ID as u16 {
            return Err(LayoutError::NodeIdCapacity);
        }
        let id = NodeId(self.next_id as u8);
        self.next_id += 1;
        Ok(id)
    }

    fn ensure_parent(&self, parent: NodeId) -> Result<(), LayoutError> {
        if self.stack_depth == 0 || self.stack[self.stack_depth - 1] != parent {
            return Err(LayoutError::InvalidParent);
        }
        Ok(())
    }

    fn ensure_child_capacity(&self, parent: NodeId) -> Result<(), LayoutError> {
        let node = self.nodes[parent.index()].ok_or(LayoutError::InvalidParent)?;
        if node.child_count >= MAX_LAYOUT_CHILDREN {
            return Err(LayoutError::ChildCapacity);
        }
        Ok(())
    }

    fn push_child(&mut self, parent: NodeId, child: ChildSpec) -> Result<(), LayoutError> {
        let node = self.nodes[parent.index()].ok_or(LayoutError::InvalidParent)?;
        let index = node.child_count;
        if index >= MAX_LAYOUT_CHILDREN {
            return Err(LayoutError::ChildCapacity);
        }
        let mut updated = node;
        updated.children[index] = Some(child);
        updated.child_count += 1;
        self.nodes[parent.index()] = Some(updated);
        Ok(())
    }

    fn push_scope(&mut self, node: NodeId) -> Result<(), LayoutError> {
        if self.stack_depth >= MAX_LAYOUT_NODES {
            return Err(LayoutError::ScopeCapacity);
        }
        self.stack[self.stack_depth] = node;
        self.stack_depth += 1;
        Ok(())
    }

    fn layout_node(&self, node_id: NodeId, rect: Rect, snapshot: &mut LayoutSnapshot) {
        let Some(node) = self.nodes[node_id.index()] else {
            return;
        };
        snapshot.set(Placement {
            id: node.id,
            kind: node.kind,
            event_id: 0,
            rect,
        });

        let content_x = rect.x + node.padding;
        let content_y = rect.y + node.padding;
        let content_width = clamp_dimension(rect.width - node.padding * 2, 1, 4000);
        let content_height = clamp_dimension(rect.height - node.padding * 2, 1, 4000);
        let mut primary_total = 0;
        for index in 0..node.child_count {
            let Some(child) = node.children[index] else {
                continue;
            };
            primary_total += match node.orientation {
                Orientation::Column => child.height,
                Orientation::Row => child.width,
            };
            if index + 1 < node.child_count {
                primary_total += node.gap;
            }
        }

        let mut cursor = match node.orientation {
            Orientation::Column => content_y,
            Orientation::Row => {
                aligned_cross_position(content_x, content_width, primary_total, node.align)
            }
        };
        for index in 0..node.child_count {
            let Some(child) = node.children[index] else {
                continue;
            };
            let (child_x, child_y, child_width, child_height) = match node.orientation {
                Orientation::Column => {
                    let child_width = if child.stretch {
                        content_width
                    } else {
                        clamp_dimension(child.width, 1, content_width)
                    };
                    let child_height = clamp_dimension(child.height, 1, content_height);
                    let child_x =
                        aligned_cross_position(content_x, content_width, child_width, node.align);
                    let child_y = cursor;
                    cursor += child_height;
                    if index + 1 < node.child_count {
                        cursor += node.gap;
                    }
                    (child_x, child_y, child_width, child_height)
                }
                Orientation::Row => {
                    let child_width = clamp_dimension(child.width, 1, content_width);
                    let child_height = if child.stretch {
                        content_height
                    } else {
                        clamp_dimension(child.height, 1, content_height)
                    };
                    let child_x = cursor;
                    let child_y = content_y + (content_height - child_height) / 2;
                    cursor += child_width;
                    if index + 1 < node.child_count {
                        cursor += node.gap;
                    }
                    (child_x, child_y, child_width, child_height)
                }
            };
            let child_rect = Rect::new(child_x, child_y, child_width, child_height);
            if self.nodes[child.id.index()].is_some() {
                self.layout_node(child.id, child_rect, snapshot);
            } else {
                snapshot.set(Placement {
                    id: child.id,
                    kind: child.kind,
                    event_id: child.event_id,
                    rect: child_rect,
                });
            }
        }
    }
}

fn normalize_viewport(width: i32, height: i32) -> (i32, i32) {
    (
        clamp_dimension(width, MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH),
        clamp_dimension(height, MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT),
    )
}

fn clamp_dimension(value: i32, minimum: i32, maximum: i32) -> i32 {
    value.clamp(minimum, maximum)
}

fn aligned_cross_position(origin: i32, available: i32, requested: i32, align: Align) -> i32 {
    let size = requested.min(available);
    match align {
        Align::Center => origin + (available - size) / 2,
        Align::End => origin + available - size,
        Align::Start => origin,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Align, ContainerSpec, InteractionError, InteractionModel, LayoutError, LayoutTree, NodeId,
        Rect, UiEventKind, WidgetKind,
    };

    #[test]
    fn retained_tree_matches_win32_column_geometry_and_resize() {
        let mut tree = LayoutTree::new(640, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(560, 260, 12, 8, Align::Start, true),
            )
            .expect("panel");
        let label = tree.label(panel, 480, 34, true).expect("label");
        let button = tree.button(panel, 180, 42, false).expect("button");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");

        let initial = tree.layout(640, 420).expect("initial layout");
        assert_eq!(
            initial.placement(root).expect("root placement").rect,
            Rect {
                x: 16,
                y: 16,
                width: 608,
                height: 388,
            }
        );
        assert_eq!(
            initial.placement(panel).expect("panel placement").rect,
            Rect {
                x: 32,
                y: 32,
                width: 576,
                height: 260,
            }
        );
        assert_eq!(
            initial.placement(label).expect("label placement").rect,
            Rect {
                x: 44,
                y: 44,
                width: 552,
                height: 34,
            }
        );
        assert_eq!(
            initial.placement(button).expect("button placement").rect,
            Rect {
                x: 44,
                y: 86,
                width: 180,
                height: 42,
            }
        );

        let resized = tree.layout(800, 420).expect("resized layout");
        assert_eq!(
            resized.placement(root).expect("root placement").rect.width,
            768
        );
        assert_eq!(
            resized
                .placement(panel)
                .expect("panel placement")
                .rect
                .width,
            736
        );
        assert_eq!(
            resized
                .placement(label)
                .expect("label placement")
                .rect
                .width,
            712
        );
        assert_eq!(
            resized.placement(button).expect("button placement").rect.x,
            44
        );
    }

    #[test]
    fn retained_tree_matches_row_alignment() {
        let mut tree = LayoutTree::new(800, 480);
        let root = tree.begin_root().expect("root");
        let row = tree
            .row(
                root,
                ContainerSpec::new(700, 52, 0, 12, Align::Center, true),
            )
            .expect("row");
        let first = tree.button(row, 160, 40, false).expect("first");
        let second = tree.button(row, 180, 40, false).expect("second");
        tree.end(row).expect("close row");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(800, 480).expect("layout");
        assert_eq!(snapshot.placement(row).expect("row").kind, WidgetKind::Row);
        assert_eq!(snapshot.placement(first).expect("first").rect.x, 224);
        assert_eq!(snapshot.placement(second).expect("second").rect.x, 396);
        assert_eq!(snapshot.placement(first).expect("first").rect.y, 38);
        assert_eq!(snapshot.placement(second).expect("second").rect.y, 38);
    }

    #[test]
    fn retained_top_bar_uses_horizontal_geometry_and_preserves_kind() {
        let mut tree = LayoutTree::new(800, 480);
        let root = tree.begin_root().expect("root");
        let top_bar = tree
            .top_bar(root, ContainerSpec::new(700, 54, 8, 12, Align::Start, true))
            .expect("top bar");
        let first = tree
            .button_with_event(top_bar, 10, 160, 38, false)
            .expect("first");
        let second = tree
            .button_with_event(top_bar, 11, 120, 38, false)
            .expect("second");
        tree.end(top_bar).expect("close top bar");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(800, 480).expect("layout");
        assert_eq!(
            snapshot.placement(top_bar).expect("top bar").kind,
            WidgetKind::TopBar
        );
        assert_eq!(snapshot.placement(first).expect("first").rect.x, 40);
        assert_eq!(snapshot.placement(second).expect("second").rect.x, 212);
        assert_eq!(snapshot.placement(first).expect("first").event_id, 10);
        assert_eq!(snapshot.placement(second).expect("second").event_id, 11);
    }

    #[test]
    fn retained_menu_trigger_uses_button_geometry_without_popup_children() {
        let mut tree = LayoutTree::new(800, 480);
        let root = tree.begin_root().expect("root");
        let top_bar = tree
            .top_bar(root, ContainerSpec::new(700, 54, 8, 12, Align::Start, true))
            .expect("top bar");
        let menu = tree.menu(top_bar, 96, 38, false).expect("menu");
        tree.end(top_bar).expect("close top bar");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(800, 480).expect("layout");
        let placement = snapshot.placement(menu).expect("menu placement");
        assert_eq!(placement.kind, WidgetKind::Menu);
        assert_eq!(placement.rect.width, 96);
        assert!(
            snapshot
                .hit_test(placement.rect.x + 4, placement.rect.y + 4)
                .is_some()
        );
    }

    #[test]
    fn scope_and_capacity_errors_are_deterministic() {
        let mut tree = LayoutTree::new(640, 420);
        assert_eq!(tree.run_ready(), Err(LayoutError::MissingRoot));
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(200, 100, 0, 0, Align::Start, false),
            )
            .expect("panel");
        assert_eq!(tree.end(root), Err(LayoutError::ScopeOrder));
        assert_eq!(
            tree.label(root, 20, 20, false),
            Err(LayoutError::InvalidParent)
        );
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");
        assert!(tree.layout(640, 420).is_ok());
        assert_eq!(NodeId::new(0), None);
        assert_eq!(NodeId::new(128), None);

        let mut children_tree = LayoutTree::new(640, 420);
        let children_root = children_tree.begin_root().expect("children root");
        for _ in 0..super::MAX_LAYOUT_CHILDREN {
            children_tree
                .label(children_root, 20, 20, false)
                .expect("bounded child");
        }
        assert_eq!(
            children_tree.label(children_root, 20, 20, false),
            Err(LayoutError::ChildCapacity)
        );

        let mut scopes_tree = LayoutTree::new(640, 420);
        let mut current = scopes_tree.begin_root().expect("scopes root");
        for _ in 0..(super::MAX_LAYOUT_NODES - 1) {
            current = scopes_tree
                .panel(
                    current,
                    ContainerSpec::new(120, 80, 0, 0, Align::Start, false),
                )
                .expect("bounded scope");
        }
        assert_eq!(
            scopes_tree.panel(
                current,
                ContainerSpec::new(120, 80, 0, 0, Align::Start, false),
            ),
            Err(LayoutError::ScopeCapacity)
        );
    }

    #[test]
    fn hit_test_and_pointer_events_preserve_source_event_id() {
        let mut tree = LayoutTree::new(640, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(400, 180, 8, 4, Align::Start, false),
            )
            .expect("panel");
        let button = tree
            .button_with_event(panel, 42, 120, 36, false)
            .expect("button");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(640, 420).expect("layout");
        let placement = snapshot.hit_test(50, 50).expect("button hit");
        assert_eq!(placement.id, button);
        assert_eq!(placement.event_id, 42);
        assert!(snapshot.hit_test(40 + 120, 50).is_none());

        let mut interaction = InteractionModel::new();
        interaction.pointer_move(&snapshot, 50, 50).expect("hover");
        interaction.pointer_down(&snapshot, 50, 50).expect("press");
        interaction.pointer_up(&snapshot, 50, 50).expect("click");
        assert_eq!(interaction.hovered(), Some(button));
        assert_eq!(interaction.pressed(), None);
        assert_eq!(interaction.focused(), Some(button));
        assert_eq!(
            interaction.events().collect::<Vec<_>>(),
            vec![
                super::UiEvent {
                    target: button,
                    event_id: 42,
                    kind: UiEventKind::HoverEnter,
                },
                super::UiEvent {
                    target: button,
                    event_id: 42,
                    kind: UiEventKind::Pressed,
                },
                super::UiEvent {
                    target: button,
                    event_id: 42,
                    kind: UiEventKind::Released,
                },
                super::UiEvent {
                    target: button,
                    event_id: 42,
                    kind: UiEventKind::Clicked,
                },
            ]
        );

        interaction.clear_events();
        interaction
            .pointer_down(&snapshot, 50, 50)
            .expect("press again");
        interaction
            .pointer_up(&snapshot, 200, 200)
            .expect("cancel outside");
        assert_eq!(interaction.pressed(), None);
        let kinds = interaction
            .events()
            .map(|event| event.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                UiEventKind::Pressed,
                UiEventKind::HoverLeave,
                UiEventKind::Released,
                UiEventKind::Cancelled,
            ]
        );

        interaction.clear_events();
        interaction
            .pointer_down(&snapshot, 50, 50)
            .expect("press for cancel");
        interaction
            .pointer_cancel(&snapshot)
            .expect("cancel capture");
        assert_eq!(interaction.pressed(), None);
        assert_eq!(
            interaction
                .events()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                UiEventKind::HoverEnter,
                UiEventKind::Pressed,
                UiEventKind::Cancelled,
            ]
        );
    }

    #[test]
    fn headless_backend_emits_frame_and_polls_native_neutral_events() {
        let mut tree = LayoutTree::new(640, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(400, 180, 8, 4, Align::Start, false),
            )
            .expect("panel");
        let button = tree
            .button_with_event(panel, 77, 120, 36, false)
            .expect("button");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(640, 420).expect("layout");
        let frame = snapshot.render_frame(240, 160);
        assert_eq!(
            frame.iter().next(),
            Some(super::RenderCommand::BeginFrame {
                width: 360,
                height: 240
            })
        );
        assert_eq!(frame.iter().last(), Some(super::RenderCommand::EndFrame));
        assert_eq!(frame.len(), snapshot.iter().count() + 2);
        assert!(!frame.is_empty());

        let button_rect = snapshot.placement(button).expect("button placement").rect;
        let x = button_rect.x + button_rect.width / 2;
        let y = button_rect.y + button_rect.height / 2;
        let mut backend = super::RetainedHeadlessBackend::new(snapshot);
        backend.pointer_move(x, y).expect("hover");
        backend.pointer_down(x, y).expect("press");
        backend.pointer_up(x, y).expect("click");

        let events = (0..4)
            .map(|_| backend.poll_event().expect("queued event"))
            .collect::<Vec<_>>();
        assert_eq!(events[0].kind, UiEventKind::HoverEnter);
        assert_eq!(events[1].kind, UiEventKind::Pressed);
        assert_eq!(events[2].kind, UiEventKind::Released);
        assert_eq!(events[3].kind, UiEventKind::Clicked);
        assert_eq!(events[3].event_id, 77);
        assert_eq!(backend.poll_event(), None);
    }

    #[test]
    fn retained_text_input_is_layouted_and_hit_testable() {
        let mut tree = LayoutTree::new(720, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(640, 240, 12, 8, Align::Start, true),
            )
            .expect("panel");
        let input = tree
            .text_input_with_event(panel, 17, 400, 38, true)
            .expect("input");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(720, 420).expect("layout");
        let placement = snapshot.hit_test(60, 60).expect("input hit");
        assert_eq!(placement.id, input);
        assert_eq!(placement.kind, WidgetKind::TextInput);
        assert_eq!(placement.event_id, 17);
        assert_eq!(placement.rect.width, 632);
    }

    #[test]
    fn retained_checkbox_is_interactive_and_preserves_event_id() {
        let mut tree = LayoutTree::new(720, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(640, 240, 12, 8, Align::Start, true),
            )
            .expect("panel");
        let checkbox = tree
            .checkbox_with_event(panel, 23, 220, 38, false)
            .expect("checkbox");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(720, 420).expect("layout");
        let placement = snapshot.hit_test(60, 60).expect("checkbox hit");
        assert_eq!(placement.id, checkbox);
        assert_eq!(placement.kind, WidgetKind::Checkbox);
        assert_eq!(placement.event_id, 23);
        assert_eq!(placement.rect.width, 220);
    }

    #[test]
    fn retained_select_is_interactive_and_resizes_with_parent() {
        let mut tree = LayoutTree::new(720, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(640, 240, 12, 8, Align::Start, true),
            )
            .expect("panel");
        let select = tree
            .select_with_event(panel, 31, 260, 38, true)
            .expect("select");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(880, 420).expect("layout");
        let placement = snapshot.hit_test(60, 60).expect("select hit");
        assert_eq!(placement.id, select);
        assert_eq!(placement.kind, WidgetKind::Select);
        assert_eq!(placement.event_id, 31);
        assert_eq!(placement.rect.width, 792);
    }

    #[test]
    fn retained_list_is_interactive_and_keeps_event_id() {
        let mut tree = LayoutTree::new(720, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(640, 240, 12, 8, Align::Start, true),
            )
            .expect("panel");
        let list = tree
            .list_with_event(panel, 41, 360, 120, true)
            .expect("list");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(720, 420).expect("layout");
        let placement = snapshot.hit_test(60, 60).expect("list hit");
        assert_eq!(placement.id, list);
        assert_eq!(placement.kind, WidgetKind::List);
        assert_eq!(placement.event_id, 41);
        assert_eq!(placement.rect.width, 632);
    }

    #[test]
    fn retained_table_is_interactive_and_resizes_with_parent() {
        let mut tree = LayoutTree::new(720, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(640, 240, 12, 8, Align::Start, true),
            )
            .expect("panel");
        let table = tree
            .table_with_event(panel, 51, 420, 140, true)
            .expect("table");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");

        let snapshot = tree.layout(880, 420).expect("layout");
        let placement = snapshot.hit_test(60, 60).expect("table hit");
        assert_eq!(placement.id, table);
        assert_eq!(placement.kind, WidgetKind::Table);
        assert_eq!(placement.event_id, 51);
        assert_eq!(placement.rect.width, 792);
    }

    #[test]
    fn pointer_event_capacity_is_bounded_without_state_corruption() {
        let mut tree = LayoutTree::new(640, 420);
        let root = tree.begin_root().expect("root");
        let panel = tree
            .panel(
                root,
                ContainerSpec::new(400, 180, 8, 4, Align::Start, false),
            )
            .expect("panel");
        let button = tree
            .button_with_event(panel, 7, 120, 36, false)
            .expect("button");
        tree.end(panel).expect("close panel");
        tree.end(root).expect("close root");
        let snapshot = tree.layout(640, 420).expect("layout");
        let mut interaction = InteractionModel::new();
        interaction
            .pointer_move(&snapshot, 50, 50)
            .expect("initial hover");
        for _ in 0..15 {
            interaction
                .pointer_move(&snapshot, 200, 200)
                .expect("leave while capacity remains");
            interaction
                .pointer_move(&snapshot, 50, 50)
                .expect("enter while capacity remains");
        }
        assert_eq!(interaction.events().count(), 31);
        interaction
            .pointer_move(&snapshot, 200, 200)
            .expect("final leave fills queue");
        assert_eq!(interaction.events().count(), super::MAX_UI_EVENTS);
        assert_eq!(
            interaction.pointer_move(&snapshot, 50, 50),
            Err(InteractionError::EventCapacity)
        );
        assert_eq!(interaction.hovered(), None);
        assert_eq!(
            interaction.events().last().expect("leave event").target,
            button
        );
    }
}
