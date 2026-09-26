//! Localized Activity and Wayback presentation adapters for first-party views.
//!
//! These render typed records into caller-owned buffers. No saved English UI
//! string or document body is needed to display history.

use crate::activity::{
    render_actor, render_empty_state, render_summary, wayback_target, ActivityAccessPolicy,
    ActivityEvent, ActivityQuery, ActivityReadStore, Actor, CheckpointId, EventId, EventResult,
    FailureCode, MetadataEntry, MetadataKey, MetadataValue, PrivacyClass, Provenance, RenderError,
    Timestamp, TransactionId, UserLocale, WaybackTarget,
};
use crate::wayback::{
    CheckpointAccessPolicy, CheckpointQuery, CheckpointReadStore, CheckpointRecord, DiffSummary,
    RestoreBackendKind, RestoreOutcome,
};
use crate::{ActivityContext, ObjectId};

pub trait ActivityTimeFormatter {
    fn format(
        &self,
        timestamp: Timestamp,
        locale: UserLocale,
        output: &mut [u8],
    ) -> Result<usize, RenderError>;
}

/// Exact, locale-neutral timestamp formatter for diagnostics and host previews.
/// Product apps should adapt the timestamp to the selected Region/Locale service.
pub struct EpochTimeFormatter;

impl ActivityTimeFormatter for EpochTimeFormatter {
    fn format(
        &self,
        timestamp: Timestamp,
        _locale: UserLocale,
        output: &mut [u8],
    ) -> Result<usize, RenderError> {
        let mut offset = append_text(output, 0, b"t=")?;
        offset = append_signed(output, offset, timestamp.seconds())?;
        offset = append_text(output, offset, b".")?;
        offset = append_decimal(output, offset, timestamp.nanos() as u64)?;
        Ok(offset)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewError {
    BufferTooSmall,
    PermissionDenied,
    EventNotFound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UndoAvailability {
    Unavailable,
    Ready,
    RequiresPreconditionCheck,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventActionAvailability {
    undo: UndoAvailability,
    wayback: Option<WaybackTarget>,
}

impl EventActionAvailability {
    pub const fn can_undo(self) -> bool {
        !matches!(self.undo, UndoAvailability::Unavailable)
    }

    pub const fn undo(self) -> UndoAvailability {
        self.undo
    }

    pub const fn wayback(self) -> Option<WaybackTarget> {
        self.wayback
    }
}

/// Describes navigable actions without executing them. Undo availability uses
/// the same preconditions as the transaction planner, so failed or irreversible
/// records never expose a misleading Undo affordance.
pub fn event_action_availability(event: &ActivityEvent) -> EventActionAvailability {
    let undo = match crate::transaction::plan_undo(event, crate::transaction::UndoPlanId(0)) {
        Ok(plan) if plan.is_conditional() => UndoAvailability::RequiresPreconditionCheck,
        Ok(_) => UndoAvailability::Ready,
        Err(_) => UndoAvailability::Unavailable,
    };
    EventActionAvailability {
        undo,
        wayback: wayback_target(event),
    }
}

impl From<RenderError> for ViewError {
    fn from(value: RenderError) -> Self {
        match value {
            RenderError::BufferTooSmall => Self::BufferTooSmall,
        }
    }
}

pub fn render_timeline<S: ActivityReadStore>(
    ledger: &S,
    filter: ActivityQuery,
    viewer: Actor,
    policy: &dyn ActivityAccessPolicy,
    time_formatter: &dyn ActivityTimeFormatter,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, ViewError> {
    render_timeline_at(
        TimelineRenderRequest {
            ledger,
            filter,
            viewer,
            policy,
            time_formatter,
            locale,
        },
        output,
        0,
    )
}

pub fn render_transaction_group<S: ActivityReadStore>(
    ledger: &S,
    transaction: TransactionId,
    viewer: Actor,
    policy: &dyn ActivityAccessPolicy,
    time_formatter: &dyn ActivityTimeFormatter,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, ViewError> {
    let filter = ActivityQuery::default().transaction(transaction);
    let count = ledger.query_visible(filter, viewer, policy).count();
    if count == 0 {
        return render_empty_state(locale, output).map_err(Into::into);
    }
    let mut offset = append_localized(
        locale,
        b"Transaction ",
        "トランザクション ".as_bytes(),
        output,
        0,
    )?;
    offset = append_decimal(output, offset, transaction.0)?;
    offset = append_localized(locale, b" (", "（".as_bytes(), output, offset)?;
    offset = append_decimal(output, offset, count as u64)?;
    offset = append_localized(
        locale,
        b" actions)\n",
        "件の操作）\n".as_bytes(),
        output,
        offset,
    )?;
    render_timeline_at(
        TimelineRenderRequest {
            ledger,
            filter,
            viewer,
            policy,
            time_formatter,
            locale,
        },
        output,
        offset,
    )
}

pub fn render_action_group<S: ActivityReadStore>(
    ledger: &S,
    group: crate::activity::ActionGroupId,
    viewer: Actor,
    policy: &dyn ActivityAccessPolicy,
    time_formatter: &dyn ActivityTimeFormatter,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, ViewError> {
    let filter = ActivityQuery::default().action_group(group);
    let count = ledger.query_visible(filter, viewer, policy).count();
    if count == 0 {
        return render_empty_state(locale, output).map_err(Into::into);
    }
    let mut offset = append_localized(
        locale,
        b"Action group #",
        "操作グループ #".as_bytes(),
        output,
        0,
    )?;
    offset = append_decimal(output, offset, group.0)?;
    offset = append_localized(locale, b" (", "（".as_bytes(), output, offset)?;
    offset = append_decimal(output, offset, count as u64)?;
    offset = append_localized(
        locale,
        b" events)\n",
        "件のイベント）\n".as_bytes(),
        output,
        offset,
    )?;
    render_timeline_at(
        TimelineRenderRequest {
            ledger,
            filter,
            viewer,
            policy,
            time_formatter,
            locale,
        },
        output,
        offset,
    )
}

pub fn render_event_detail<S: ActivityReadStore>(
    ledger: &S,
    id: EventId,
    viewer: Actor,
    policy: &dyn ActivityAccessPolicy,
    time_formatter: &dyn ActivityTimeFormatter,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, ViewError> {
    let event = ledger
        .get_visible(id, viewer, policy)
        .ok_or(ViewError::EventNotFound)?;
    let mut offset = 0;
    offset = append_label(locale, b"Actor: ", "実行者: ".as_bytes(), output, offset)?;
    let length = render_actor(event.actor(), locale, &mut output[offset..])?;
    offset += length;
    offset = append_text(output, offset, b" #")?;
    offset = append_decimal(output, offset, event.actor().id().0)?;
    offset = append_text(output, offset, b"\n")?;

    offset = append_context(event.context(), event.device_id(), locale, output, offset)?;

    offset = append_label(locale, b"Time: ", "時刻: ".as_bytes(), output, offset)?;
    offset += time_formatter.format(event.occurred_at(), locale, &mut output[offset..])?;
    offset = append_text(output, offset, b"\n")?;

    offset = append_label(locale, b"Action: ", "操作: ".as_bytes(), output, offset)?;
    offset += render_summary(event, locale, &mut output[offset..])?;
    offset = append_text(output, offset, b"\n")?;
    offset = append_label(locale, b"Result: ", "結果: ".as_bytes(), output, offset)?;
    offset += render_result(event.result(), locale, &mut output[offset..])?;
    offset = append_text(output, offset, b"\n")?;

    if let Some(group) = event.action_group_id() {
        offset = append_label(
            locale,
            b"Action group: ",
            "操作グループ: ".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, group.0)?;
        offset = append_text(output, offset, b"\n")?;
    }

    if let Some(workspace) = event.context().workspace_id {
        offset = append_label(
            locale,
            b"Workspace: ",
            "ワークスペース: ".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, workspace.0)?;
        offset = append_text(output, offset, b"\n")?;
    }
    offset = append_ids(
        locale,
        b"Targets: ",
        "対象: ".as_bytes(),
        event.targets(),
        output,
        offset,
    )?;
    offset = append_ids(
        locale,
        b"Sources: ",
        "参照元: ".as_bytes(),
        event.sources(),
        output,
        offset,
    )?;

    if let Some(transaction) = event.transaction_id() {
        offset = append_label(
            locale,
            b"Transaction: ",
            "トランザクション: ".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, transaction.0)?;
        offset = append_text(output, offset, b"\n")?;
    }
    if let Some(correlation) = event.correlation_id() {
        offset = append_label(
            locale,
            b"Correlation: ",
            "関連ID: ".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, correlation.0)?;
        offset = append_text(output, offset, b"\n")?;
    }
    let actions = event_action_availability(event);
    offset = append_event_actions(actions, locale, output, offset)?;
    if let Some(checkpoint) = event.checkpoint_before() {
        offset = append_checkpoint_id(
            locale,
            b"Checkpoint before: ",
            "変更前チェックポイント: ".as_bytes(),
            checkpoint,
            output,
            offset,
        )?;
    }
    if let Some(checkpoint) = event.checkpoint_after() {
        offset = append_checkpoint_id(
            locale,
            b"Checkpoint after: ",
            "変更後チェックポイント: ".as_bytes(),
            checkpoint,
            output,
            offset,
        )?;
    }
    offset = append_provenance(event.provenance(), locale, output, offset)?;
    for metadata in event.metadata() {
        offset = render_metadata(metadata, locale, output, offset)?;
    }
    Ok(offset)
}

fn append_context(
    context: ActivityContext,
    device: Option<crate::activity::DeviceId>,
    locale: UserLocale,
    output: &mut [u8],
    mut offset: usize,
) -> Result<usize, ViewError> {
    offset = append_label(locale, b"App: ", "アプリ: ".as_bytes(), output, offset)?;
    offset = append_decimal(output, offset, context.app_id.0)?;
    offset = append_label(
        locale,
        b" | Session: ",
        " | セッション: ".as_bytes(),
        output,
        offset,
    )?;
    offset = append_decimal(output, offset, context.app_session_id.0)?;
    offset = append_label(
        locale,
        b" | Node: ",
        " | ノード: ".as_bytes(),
        output,
        offset,
    )?;
    offset = append_decimal(output, offset, context.node_id.0)?;
    if let Some(surface) = context.surface_id {
        offset = append_label(
            locale,
            b" | Surface: ",
            " | サーフェス: ".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, surface.0)?;
    }
    if let Some(device) = device {
        offset = append_label(
            locale,
            b" | Device: ",
            " | デバイス: ".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, device.0)?;
    }
    offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
    Ok(offset)
}

fn append_event_actions(
    actions: EventActionAvailability,
    locale: UserLocale,
    output: &mut [u8],
    mut offset: usize,
) -> Result<usize, ViewError> {
    offset = append_label(
        locale,
        b"Available actions: ",
        "利用可能な操作: ".as_bytes(),
        output,
        offset,
    )?;
    let mut has_action = false;
    match actions.undo() {
        UndoAvailability::Unavailable => {}
        UndoAvailability::Ready => {
            offset = append_localized(locale, b"Undo", "取り消し".as_bytes(), output, offset)?;
            has_action = true;
        }
        UndoAvailability::RequiresPreconditionCheck => {
            offset = append_localized(
                locale,
                b"Undo (check conditions)",
                "取り消し（条件確認が必要）".as_bytes(),
                output,
                offset,
            )?;
            has_action = true;
        }
    }
    if actions.wayback().is_some() {
        if has_action {
            offset = append_text(output, offset, b", ").map_err(ViewError::from)?;
        }
        offset = append_localized(
            locale,
            b"Open in Wayback",
            "Waybackで開く".as_bytes(),
            output,
            offset,
        )?;
        has_action = true;
    }
    if !has_action {
        offset = append_localized(locale, b"None", "ありません".as_bytes(), output, offset)?;
    }
    append_text(output, offset, b"\n").map_err(ViewError::from)
}

/// Render a payload-free comparison summary for the Wayback preview pane.
pub fn render_diff_summary(
    object: ObjectId,
    summary: DiffSummary,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, RenderError> {
    let mut offset = append_localized(locale, b"Object #", "オブジェクト #".as_bytes(), output, 0)?;
    offset = append_decimal(output, offset, object.0)?;
    offset = append_text(output, offset, b": ")?;
    match summary {
        DiffSummary::Unchanged => {
            append_localized(locale, b"No changes", "変更なし".as_bytes(), output, offset)
        }
        DiffSummary::Changed {
            current_bytes,
            target_bytes,
        } => {
            let label = append_localized(
                locale,
                b"Changed (current ",
                "変更あり（現在 ".as_bytes(),
                output,
                offset,
            )?;
            let label = append_decimal(output, label, current_bytes as u64)?;
            let label = append_localized(
                locale,
                b" bytes, target ",
                " バイト、対象 ".as_bytes(),
                output,
                label,
            )?;
            let label = append_decimal(output, label, target_bytes as u64)?;
            append_localized(locale, b" bytes)", " バイト）".as_bytes(), output, label)
        }
        DiffSummary::MissingCurrent { target_bytes } => {
            let label = append_localized(
                locale,
                b"Current object is missing; target has ",
                "現在のオブジェクトはありません。対象サイズ: ".as_bytes(),
                output,
                offset,
            )?;
            let label = append_decimal(output, label, target_bytes as u64)?;
            append_localized(locale, b" bytes", " バイト".as_bytes(), output, label)
        }
    }
}

pub fn render_activity_error(locale: UserLocale, output: &mut [u8]) -> Result<usize, RenderError> {
    let message: &[u8] = match locale {
        UserLocale::EnUs => b"Activity is unavailable. Try again later.",
        UserLocale::JaJp => {
            "アクティビティを利用できません。しばらくしてから再試行してください。".as_bytes()
        }
    };
    copy_text(output, 0, message)
}

pub fn render_access_denied(locale: UserLocale, output: &mut [u8]) -> Result<usize, RenderError> {
    let message: &[u8] = match locale {
        UserLocale::EnUs => b"You do not have permission to view this activity.",
        UserLocale::JaJp => "このアクティビティを表示する権限がありません。".as_bytes(),
    };
    copy_text(output, 0, message)
}

pub fn render_checkpoint_timeline<S: CheckpointReadStore>(
    store: &S,
    filter: CheckpointQuery,
    viewer: Actor,
    policy: &dyn CheckpointAccessPolicy,
    time_formatter: &dyn ActivityTimeFormatter,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, ViewError> {
    let mut offset = 0;
    let mut count = 0usize;
    for checkpoint in CheckpointReadStore::query_visible(store, filter, viewer, policy) {
        if count > 0 {
            offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
        }
        offset = render_checkpoint_row(checkpoint, time_formatter, locale, output, offset)?;
        count += 1;
    }
    if count == 0 {
        return render_empty_state(locale, output).map_err(Into::into);
    }
    Ok(offset)
}

pub fn render_checkpoint_detail(
    checkpoint: &CheckpointRecord,
    viewer: Actor,
    policy: &dyn CheckpointAccessPolicy,
    time_formatter: &dyn ActivityTimeFormatter,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, ViewError> {
    if !policy.can_read(viewer, checkpoint) {
        return Err(ViewError::PermissionDenied);
    }
    let mut offset = crate::wayback::render_checkpoint_marker(checkpoint, locale, output)?;
    offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
    offset = append_label(locale, b"Time: ", "時刻: ".as_bytes(), output, offset)?;
    offset += time_formatter.format(checkpoint.created_at(), locale, &mut output[offset..])?;
    offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
    offset = append_label(locale, b"Scope: ", "範囲: ".as_bytes(), output, offset)?;
    offset = append_scope(checkpoint, locale, output, offset)?;
    offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
    offset = append_label(
        locale,
        b"Objects: ",
        "オブジェクト数: ".as_bytes(),
        output,
        offset,
    )?;
    offset = append_decimal(output, offset, checkpoint.objects().count() as u64)?;
    offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
    for object in checkpoint.objects() {
        offset = append_label(
            locale,
            b"Object #",
            "オブジェクト #".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, object.object_id().0)?;
        offset = append_localized(
            locale,
            b" at revision #",
            " のrevision #".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, object.revision_id().0)?;
        offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
    }
    offset = append_label(
        locale,
        b"Backend reference: ",
        "バックエンド参照: ".as_bytes(),
        output,
        offset,
    )?;
    offset = append_decimal(output, offset, checkpoint.backend_ref().0)?;
    offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
    offset = append_label(locale, b"Status: ", "状態: ".as_bytes(), output, offset)?;
    offset = append_checkpoint_status(checkpoint, locale, output, offset)?;
    Ok(offset)
}

struct TimelineRenderRequest<'a, S> {
    ledger: &'a S,
    filter: ActivityQuery,
    viewer: Actor,
    policy: &'a dyn ActivityAccessPolicy,
    time_formatter: &'a dyn ActivityTimeFormatter,
    locale: UserLocale,
}

fn render_timeline_at<S: ActivityReadStore>(
    request: TimelineRenderRequest<'_, S>,
    output: &mut [u8],
    mut offset: usize,
) -> Result<usize, ViewError> {
    let mut count = 0usize;
    for event in request
        .ledger
        .query_visible(request.filter, request.viewer, request.policy)
    {
        offset = render_event_row(
            event,
            request.time_formatter,
            request.locale,
            output,
            offset,
        )?;
        count += 1;
    }
    if count == 0 && offset == 0 {
        return render_empty_state(request.locale, output).map_err(Into::into);
    }
    Ok(offset)
}

fn render_event_row(
    event: &ActivityEvent,
    time_formatter: &dyn ActivityTimeFormatter,
    locale: UserLocale,
    output: &mut [u8],
    mut offset: usize,
) -> Result<usize, ViewError> {
    offset += time_formatter.format(event.occurred_at(), locale, &mut output[offset..])?;
    offset = append_text(output, offset, b" | ").map_err(ViewError::from)?;
    offset += render_actor(event.actor(), locale, &mut output[offset..])?;
    offset = append_text(output, offset, b" | ").map_err(ViewError::from)?;
    offset += render_summary(event, locale, &mut output[offset..])?;
    offset = append_text(output, offset, b" | ").map_err(ViewError::from)?;
    offset += render_result(event.result(), locale, &mut output[offset..])?;
    offset = append_ids_inline(
        locale,
        b" | Objects: ",
        " | 対象: ".as_bytes(),
        event.targets(),
        output,
        offset,
    )?;
    if let Some(transaction) = event.transaction_id() {
        offset = append_localized(locale, b" | Tx ", " | Tx ".as_bytes(), output, offset)?;
        offset = append_decimal(output, offset, transaction.0)?;
    }
    append_text(output, offset, b"\n").map_err(ViewError::from)
}

fn render_checkpoint_row(
    checkpoint: &CheckpointRecord,
    time_formatter: &dyn ActivityTimeFormatter,
    locale: UserLocale,
    output: &mut [u8],
    mut offset: usize,
) -> Result<usize, ViewError> {
    offset += time_formatter.format(checkpoint.created_at(), locale, &mut output[offset..])?;
    offset = append_text(output, offset, b" | ").map_err(ViewError::from)?;
    offset += crate::wayback::render_checkpoint_marker(checkpoint, locale, &mut output[offset..])?;
    offset = append_text(output, offset, b" #").map_err(ViewError::from)?;
    offset = append_decimal(output, offset, checkpoint.id().0)?;
    offset = append_text(output, offset, b" | ").map_err(ViewError::from)?;
    offset = append_scope(checkpoint, locale, output, offset)?;
    offset = append_text(output, offset, b" | ").map_err(ViewError::from)?;
    offset = append_decimal(output, offset, checkpoint.objects().count() as u64)?;
    offset = append_localized(locale, b" objects", " 個".as_bytes(), output, offset)?;
    Ok(offset)
}

pub fn render_result(
    result: EventResult,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, RenderError> {
    let offset = match result {
        EventResult::Succeeded => {
            append_localized(locale, b"Completed", "完了".as_bytes(), output, 0)?
        }
        EventResult::Pending => {
            append_localized(locale, b"In progress", "処理中".as_bytes(), output, 0)?
        }
        EventResult::Failed(code) => {
            let prefix = append_localized(locale, b"Failed: ", "失敗: ".as_bytes(), output, 0)?;
            append_failure(code, locale, output, prefix)?
        }
        EventResult::Partial { succeeded, failed } => {
            let mut offset =
                append_localized(locale, b"Partial (", "一部完了（".as_bytes(), output, 0)?;
            offset = append_decimal(output, offset, succeeded as u64)?;
            offset = append_localized(locale, b" done, ", " 完了、".as_bytes(), output, offset)?;
            offset = append_decimal(output, offset, failed as u64)?;
            append_localized(locale, b" failed)", " 失敗）".as_bytes(), output, offset)?
        }
    };
    if offset > output.len() {
        return Err(RenderError::BufferTooSmall);
    }
    Ok(offset)
}

fn render_metadata(
    metadata: MetadataEntry,
    locale: UserLocale,
    output: &mut [u8],
    mut offset: usize,
) -> Result<usize, ViewError> {
    offset = append_label(
        locale,
        b"Metadata: ",
        "メタデータ: ".as_bytes(),
        output,
        offset,
    )?;
    let (key_en, key_ja) = match metadata.key() {
        MetadataKey::ItemCount => (b"item count".as_slice(), "項目数".as_bytes()),
        MetadataKey::ByteCount => (b"byte count".as_slice(), "バイト数".as_bytes()),
        MetadataKey::ContentTypeCode => (
            b"content type code".as_slice(),
            "コンテンツ種別コード".as_bytes(),
        ),
        MetadataKey::FailureCode => (b"failure code".as_slice(), "失敗コード".as_bytes()),
        MetadataKey::Credential => (b"credential".as_slice(), "認証情報".as_bytes()),
        MetadataKey::Password => (b"password".as_slice(), "パスワード".as_bytes()),
        MetadataKey::AccessToken => (b"access token".as_slice(), "アクセストークン".as_bytes()),
        MetadataKey::AuthenticationData => {
            (b"authentication data".as_slice(), "認証データ".as_bytes())
        }
        MetadataKey::DocumentContent => (
            b"document content".as_slice(),
            "ドキュメント本文".as_bytes(),
        ),
        MetadataKey::SensitivePayload => (b"sensitive payload".as_slice(), "機密データ".as_bytes()),
        MetadataKey::ConfirmedByActor => (b"confirmed by actor".as_slice(), "確認者".as_bytes()),
        MetadataKey::RestoreMode => (b"restore mode".as_slice(), "復元方法".as_bytes()),
        MetadataKey::Other(_) => (
            b"other metadata".as_slice(),
            "その他のメタデータ".as_bytes(),
        ),
    };
    offset = append_localized(locale, key_en, key_ja, output, offset)?;
    offset = append_text(output, offset, b" = ").map_err(ViewError::from)?;
    offset = match metadata.value() {
        MetadataValue::Count(value) => append_decimal(output, offset, value as u64)?,
        MetadataValue::Bytes(value) => append_decimal(output, offset, value)?,
        MetadataValue::Code(1) if metadata.key() == MetadataKey::RestoreMode => append_localized(
            locale,
            b"in-place",
            "元の場所へ復元".as_bytes(),
            output,
            offset,
        )?,
        MetadataValue::Code(2) if metadata.key() == MetadataKey::RestoreMode => append_localized(
            locale,
            b"restore as copy",
            "コピーとして復元".as_bytes(),
            output,
            offset,
        )?,
        MetadataValue::Code(value) => append_decimal(output, offset, value as u64)?,
        MetadataValue::Flag(value) => match (locale, value) {
            (UserLocale::EnUs, true) => append_text(output, offset, b"yes")?,
            (UserLocale::EnUs, false) => append_text(output, offset, b"no")?,
            (UserLocale::JaJp, true) => append_text(output, offset, "はい".as_bytes())?,
            (UserLocale::JaJp, false) => append_text(output, offset, "いいえ".as_bytes())?,
        },
        MetadataValue::ActorReference(actor) => append_decimal(output, offset, actor.0)?,
        MetadataValue::Redacted => {
            append_localized(locale, b"[redacted]", "[秘匿]".as_bytes(), output, offset)?
        }
    };
    offset = append_localized(locale, b" (", "（".as_bytes(), output, offset)?;
    let privacy: &[u8] = match (locale, metadata.privacy()) {
        (UserLocale::EnUs, PrivacyClass::PublicMetadata) => b"public",
        (UserLocale::EnUs, PrivacyClass::Sensitive) => b"sensitive",
        (UserLocale::EnUs, PrivacyClass::Secret) => b"secret",
        (UserLocale::EnUs, PrivacyClass::Content) => b"content",
        (UserLocale::JaJp, PrivacyClass::PublicMetadata) => "公開情報".as_bytes(),
        (UserLocale::JaJp, PrivacyClass::Sensitive) => "機微情報".as_bytes(),
        (UserLocale::JaJp, PrivacyClass::Secret) => "秘密情報".as_bytes(),
        (UserLocale::JaJp, PrivacyClass::Content) => "本文".as_bytes(),
    };
    offset = append_text(output, offset, privacy).map_err(ViewError::from)?;
    offset = append_localized(locale, b")\n", "）\n".as_bytes(), output, offset)?;
    Ok(offset)
}

pub fn render_restore_outcome(
    outcome: RestoreOutcome,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, RenderError> {
    let (en, ja) = if !outcome.backend_executed() {
        (
            b"Restore was not applied: ".as_slice(),
            "復元は適用されませんでした: ".as_bytes(),
        )
    } else {
        match outcome.backend() {
            RestoreBackendKind::NagiTarget => (
                b"Nagi target restore result: ".as_slice(),
                "Nagi実機復元の結果: ".as_bytes(),
            ),
            RestoreBackendKind::HostInMemorySandbox => (
                b"Host in-memory sandbox result (not target restore): ".as_slice(),
                "ホスト内メモリsandboxの結果（実機復元ではありません）: ".as_bytes(),
            ),
        }
    };
    let mut offset = append_localized(locale, en, ja, output, 0)?;
    offset = offset
        .checked_add(render_result(
            outcome.result(),
            locale,
            &mut output[offset..],
        )?)
        .ok_or(RenderError::BufferTooSmall)?;
    let mut created = outcome.created_objects();
    if let Some(first) = created.next() {
        offset = append_localized(
            locale,
            b"\nCreated copies: ",
            "\n作成したコピー: ".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, first.0)?;
        for object in created {
            offset = append_text(output, offset, b", ")?;
            offset = append_decimal(output, offset, object.0)?;
        }
    }
    if let Some(checkpoint) = outcome.recovery_checkpoint() {
        offset = append_localized(
            locale,
            b"\nRecovery checkpoint #",
            "\n復旧用checkpoint #".as_bytes(),
            output,
            offset,
        )?;
        offset = append_decimal(output, offset, checkpoint.0)?;
    }
    Ok(offset)
}

fn append_failure(
    failure: FailureCode,
    locale: UserLocale,
    output: &mut [u8],
    offset: usize,
) -> Result<usize, RenderError> {
    let (en, ja) = match failure {
        FailureCode::PermissionDenied => (
            b"permission denied".as_slice(),
            "権限がありません".as_bytes(),
        ),
        FailureCode::StaleState => (
            b"state changed".as_slice(),
            "状態が変更されています".as_bytes(),
        ),
        FailureCode::MissingObject => (
            b"object unavailable".as_slice(),
            "オブジェクトを利用できません".as_bytes(),
        ),
        FailureCode::MissingCheckpoint => (
            b"checkpoint unavailable".as_slice(),
            "チェックポイントを利用できません".as_bytes(),
        ),
        FailureCode::Unsupported => (b"unsupported".as_slice(), "未対応です".as_bytes()),
        FailureCode::BackendFailure => (
            b"backend failed".as_slice(),
            "バックエンドに失敗しました".as_bytes(),
        ),
        FailureCode::Capacity => (
            b"capacity reached".as_slice(),
            "上限に達しました".as_bytes(),
        ),
        FailureCode::Validation => (
            b"validation failed".as_slice(),
            "検証に失敗しました".as_bytes(),
        ),
        FailureCode::Other(_) => (
            b"operation failed".as_slice(),
            "操作に失敗しました".as_bytes(),
        ),
    };
    append_localized(locale, en, ja, output, offset)
}

fn append_provenance(
    provenance: Provenance,
    locale: UserLocale,
    output: &mut [u8],
    mut offset: usize,
) -> Result<usize, ViewError> {
    match provenance {
        Provenance::Direct {
            originating_intent: Some(intent),
        } => {
            offset = append_label(locale, b"Intent: ", "意図ID: ".as_bytes(), output, offset)?;
            offset = append_decimal(output, offset, intent.0)?;
            offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
        }
        Provenance::Direct {
            originating_intent: None,
        } => {}
        Provenance::Delegated {
            requested_by,
            originating_intent,
            delegated_authority,
        } => {
            offset = append_label(
                locale,
                b"Requested by: ",
                "依頼者: ".as_bytes(),
                output,
                offset,
            )?;
            offset += render_actor(requested_by, locale, &mut output[offset..])?;
            offset = append_text(output, offset, b" #").map_err(ViewError::from)?;
            offset = append_decimal(output, offset, requested_by.id().0)?;
            offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
            offset = append_label(locale, b"Intent: ", "意図ID: ".as_bytes(), output, offset)?;
            offset = append_decimal(output, offset, originating_intent.0)?;
            offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
            offset = append_label(
                locale,
                b"Delegated authority ref: ",
                "委譲権限参照: ".as_bytes(),
                output,
                offset,
            )?;
            offset = append_decimal(output, offset, delegated_authority.0)?;
            offset = append_text(output, offset, b"\n").map_err(ViewError::from)?;
        }
    }
    Ok(offset)
}

fn append_ids(
    locale: UserLocale,
    en_label: &[u8],
    ja_label: &[u8],
    ids: impl Iterator<Item = ObjectId>,
    output: &mut [u8],
    offset: usize,
) -> Result<usize, ViewError> {
    let offset = append_ids_inline(locale, en_label, ja_label, ids, output, offset)?;
    append_text(output, offset, b"\n").map_err(ViewError::from)
}

fn append_ids_inline(
    locale: UserLocale,
    en_label: &[u8],
    ja_label: &[u8],
    ids: impl Iterator<Item = ObjectId>,
    output: &mut [u8],
    mut offset: usize,
) -> Result<usize, ViewError> {
    let ids: [Option<ObjectId>; 4] = {
        let mut values = [None; 4];
        for (index, id) in ids.take(values.len()).enumerate() {
            values[index] = Some(id);
        }
        values
    };
    offset = append_localized(locale, en_label, ja_label, output, offset)?;
    let mut first = true;
    for id in ids.iter().filter_map(|id| *id) {
        if !first {
            offset = append_text(output, offset, b",").map_err(ViewError::from)?;
        }
        offset = append_decimal(output, offset, id.0)?;
        first = false;
    }
    if first {
        offset = append_localized(locale, b"none", "なし".as_bytes(), output, offset)?;
    }
    Ok(offset)
}

fn append_checkpoint_id(
    locale: UserLocale,
    en: &[u8],
    ja: &[u8],
    id: CheckpointId,
    output: &mut [u8],
    offset: usize,
) -> Result<usize, ViewError> {
    let offset = append_localized(locale, en, ja, output, offset)?;
    let offset = append_decimal(output, offset, id.0)?;
    append_text(output, offset, b"\n").map_err(ViewError::from)
}

fn append_scope(
    checkpoint: &CheckpointRecord,
    locale: UserLocale,
    output: &mut [u8],
    offset: usize,
) -> Result<usize, ViewError> {
    let (en, ja) = match checkpoint.scope() {
        crate::wayback::CheckpointScope::Document => {
            (b"Document".as_slice(), "ドキュメント".as_bytes())
        }
        crate::wayback::CheckpointScope::Workspace => {
            (b"Workspace".as_slice(), "ワークスペース".as_bytes())
        }
        crate::wayback::CheckpointScope::Transaction => {
            (b"Transaction".as_slice(), "トランザクション".as_bytes())
        }
        crate::wayback::CheckpointScope::System => (b"System".as_slice(), "システム".as_bytes()),
    };
    append_localized(locale, en, ja, output, offset).map_err(Into::into)
}

fn append_checkpoint_status(
    checkpoint: &CheckpointRecord,
    locale: UserLocale,
    output: &mut [u8],
    offset: usize,
) -> Result<usize, ViewError> {
    let (en, ja) = match checkpoint.validity() {
        crate::wayback::CheckpointValidity::Available => {
            (b"Available".as_slice(), "利用可能".as_bytes())
        }
        crate::wayback::CheckpointValidity::Expired => {
            (b"Expired".as_slice(), "期限切れ".as_bytes())
        }
        crate::wayback::CheckpointValidity::Unavailable => {
            (b"Unavailable".as_slice(), "利用不可".as_bytes())
        }
        crate::wayback::CheckpointValidity::Corrupt => (b"Invalid".as_slice(), "無効".as_bytes()),
    };
    append_localized(locale, en, ja, output, offset).map_err(Into::into)
}

fn append_label(
    locale: UserLocale,
    en: &[u8],
    ja: &[u8],
    output: &mut [u8],
    offset: usize,
) -> Result<usize, ViewError> {
    append_localized(locale, en, ja, output, offset).map_err(Into::into)
}

fn append_localized(
    locale: UserLocale,
    en: &[u8],
    ja: &[u8],
    output: &mut [u8],
    offset: usize,
) -> Result<usize, RenderError> {
    let text = match locale {
        UserLocale::EnUs => en,
        UserLocale::JaJp => ja,
    };
    append_text(output, offset, text)
}

fn append_signed(output: &mut [u8], offset: usize, value: i64) -> Result<usize, RenderError> {
    if value < 0 {
        let offset = append_text(output, offset, b"-")?;
        append_decimal(output, offset, value.unsigned_abs())
    } else {
        append_decimal(output, offset, value as u64)
    }
}

fn append_decimal(output: &mut [u8], offset: usize, mut value: u64) -> Result<usize, RenderError> {
    let mut reverse = [0u8; 20];
    let mut length = 0;
    loop {
        reverse[length] = b'0' + (value % 10) as u8;
        length += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    if output.len().saturating_sub(offset) < length {
        return Err(RenderError::BufferTooSmall);
    }
    for index in 0..length {
        output[offset + index] = reverse[length - index - 1];
    }
    Ok(offset + length)
}

fn append_text(output: &mut [u8], offset: usize, text: &[u8]) -> Result<usize, RenderError> {
    copy_text(output, offset, text)
}

fn copy_text(output: &mut [u8], offset: usize, text: &[u8]) -> Result<usize, RenderError> {
    if output.len().saturating_sub(offset) < text.len() {
        return Err(RenderError::BufferTooSmall);
    }
    output[offset..offset + text.len()].copy_from_slice(text);
    Ok(offset + text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{
        ActionGroupId, ActivityDraft, ActivityLedger, ActivityQuery, ActorId, ActorKind, DeviceId,
        InverseKind, MetadataKey, MetadataValue, PrivacyClass, Provenance, Reversibility,
        RevisionId, TransactionId, UndoDescriptor,
    };
    use crate::wayback::{
        CheckpointMutationPolicy, CheckpointObject, CheckpointOrigin, CheckpointReason,
        CheckpointScope, CheckpointStore, CheckpointValidity, SnapshotBackendRef,
    };
    use crate::{ActivityContext, AppId, AppSessionId, NodeId, WorkspaceId};

    const CONTEXT: ActivityContext = ActivityContext {
        app_id: AppId(1),
        app_session_id: AppSessionId(2),
        node_id: NodeId(3),
        surface_id: None,
        workspace_id: Some(WorkspaceId(4)),
    };
    const USER: Actor = Actor::new(ActorId(1), ActorKind::User);

    fn time(seconds: i64) -> Timestamp {
        Timestamp::new(seconds, 0).unwrap()
    }

    struct Visible;
    impl ActivityAccessPolicy for Visible {
        fn can_read(&self, _viewer: Actor, _event: &ActivityEvent) -> bool {
            true
        }
    }
    impl CheckpointAccessPolicy for Visible {
        fn can_read(&self, _viewer: Actor, _checkpoint: &CheckpointRecord) -> bool {
            true
        }
    }

    #[test]
    fn timeline_groups_by_transaction_filters_and_renders_actor_time_summary_result_and_targets() {
        let mut ledger = ActivityLedger::new();
        let transaction = TransactionId(9);
        let id = ledger
            .append(
                ActivityDraft::new(
                    time(17),
                    USER,
                    CONTEXT,
                    crate::activity::ActionKind::ObjectChanged,
                )
                .with_target(ObjectId(88))
                .unwrap()
                .with_device(DeviceId(5))
                .with_action_group(ActionGroupId(10))
                .with_transaction(transaction)
                .with_result(EventResult::Succeeded)
                .with_metadata(
                    MetadataKey::ItemCount,
                    MetadataValue::Count(1),
                    PrivacyClass::PublicMetadata,
                )
                .unwrap(),
            )
            .unwrap();
        let mut output = [0; 256];
        let length = render_transaction_group(
            &ledger,
            transaction,
            USER,
            &Visible,
            &EpochTimeFormatter,
            UserLocale::EnUs,
            &mut output,
        )
        .unwrap();
        let text = core::str::from_utf8(&output[..length]).unwrap();
        assert!(text.contains("Transaction 9 (1 actions)"));
        assert!(text.contains("t=17.0 | You | Changed an item | Completed | Objects: 88"));

        let mut detail = [0; 768];
        let detail_len = render_event_detail(
            &ledger,
            id,
            USER,
            &Visible,
            &EpochTimeFormatter,
            UserLocale::JaJp,
            &mut detail,
        )
        .unwrap();
        let detail = core::str::from_utf8(&detail[..detail_len]).unwrap();
        assert!(detail.contains("実行者: あなた #1"));
        assert!(detail.contains("時刻: t=17.0"));
        assert!(detail.contains("対象: 88"));
        assert!(detail.contains("トランザクション: 9"));
        assert!(detail.contains("アプリ: 1 | セッション: 2 | ノード: 3 | デバイス: 5"));
        assert!(detail.contains("操作グループ: 10"));
    }

    #[test]
    fn detail_and_checkpoint_views_fail_closed_on_visibility_policy() {
        struct Hidden;
        impl ActivityAccessPolicy for Hidden {
            fn can_read(&self, _viewer: Actor, _event: &ActivityEvent) -> bool {
                false
            }
        }
        impl CheckpointAccessPolicy for Hidden {
            fn can_read(&self, _viewer: Actor, _checkpoint: &CheckpointRecord) -> bool {
                false
            }
        }
        let mut ledger = ActivityLedger::new();
        let id = ledger
            .append(ActivityDraft::new(
                time(1),
                USER,
                CONTEXT,
                crate::activity::ActionKind::ObjectAccessed,
            ))
            .unwrap();
        assert_eq!(
            render_event_detail(
                &ledger,
                id,
                USER,
                &Hidden,
                &EpochTimeFormatter,
                UserLocale::EnUs,
                &mut [0; 128],
            ),
            Err(ViewError::EventNotFound)
        );
        let mut denied = [0; 64];
        let denied_len = render_access_denied(UserLocale::EnUs, &mut denied).unwrap();
        assert!(core::str::from_utf8(&denied[..denied_len])
            .unwrap()
            .contains("permission"));

        let mut checkpoints = CheckpointStore::new();
        let mut activity = ActivityLedger::new();
        let draft = crate::wayback::CheckpointDraft::new(
            time(1),
            USER,
            CONTEXT,
            CheckpointScope::Document,
            CheckpointOrigin::User,
            CheckpointReason::UserRequested,
            SnapshotBackendRef(1),
        )
        .with_object(CheckpointObject::new(ObjectId(9), RevisionId(1)))
        .unwrap();
        let (checkpoint_id, _) = checkpoints.create(draft, &mut activity).unwrap();
        let checkpoint = checkpoints.get(checkpoint_id).unwrap();
        assert_eq!(checkpoint.validity(), CheckpointValidity::Available);
        assert_eq!(
            render_checkpoint_detail(
                checkpoint,
                USER,
                &Hidden,
                &EpochTimeFormatter,
                UserLocale::EnUs,
                &mut [0; 128],
            ),
            Err(ViewError::PermissionDenied)
        );
    }

    #[test]
    fn empty_and_error_states_are_localized_and_filter_results_do_not_leak_denied_events() {
        let mut ledger = ActivityLedger::new();
        ledger
            .append(ActivityDraft::new(
                time(1),
                USER,
                CONTEXT,
                crate::activity::ActionKind::ObjectCreated,
            ))
            .unwrap();
        struct Deny;
        impl ActivityAccessPolicy for Deny {
            fn can_read(&self, _viewer: Actor, _event: &ActivityEvent) -> bool {
                false
            }
        }
        let mut output = [0; 128];
        let length = render_timeline(
            &ledger,
            ActivityQuery::default(),
            USER,
            &Deny,
            &EpochTimeFormatter,
            UserLocale::JaJp,
            &mut output,
        )
        .unwrap();
        assert_eq!(&output[..length], "履歴はまだありません。".as_bytes());
        let length = render_activity_error(UserLocale::JaJp, &mut output).unwrap();
        assert!(core::str::from_utf8(&output[..length])
            .unwrap()
            .contains("利用できません"));
    }

    #[test]
    fn checkpoint_timeline_shows_pins_scope_and_time() {
        let mut store = CheckpointStore::new();
        let mut activity = ActivityLedger::new();
        let draft = crate::wayback::CheckpointDraft::new(
            time(31),
            USER,
            CONTEXT,
            CheckpointScope::Document,
            CheckpointOrigin::User,
            CheckpointReason::UserRequested,
            SnapshotBackendRef(11),
        )
        .with_object(CheckpointObject::new(ObjectId(17), RevisionId(3)))
        .unwrap();
        let (checkpoint_id, _) = store.create(draft, &mut activity).unwrap();
        struct AllowPin;
        impl CheckpointMutationPolicy for AllowPin {
            fn can_change_pin(
                &self,
                _actor: Actor,
                _checkpoint: &CheckpointRecord,
                _pinned: bool,
            ) -> bool {
                true
            }
        }
        store
            .set_pinned(
                crate::wayback::CheckpointPinRequest::new(
                    checkpoint_id,
                    true,
                    time(31),
                    USER,
                    Provenance::Direct {
                        originating_intent: None,
                    },
                ),
                &AllowPin,
                &mut activity,
            )
            .unwrap();
        let mut output = [0; 128];
        let length = render_checkpoint_timeline(
            &store,
            CheckpointQuery::default().workspace(WorkspaceId(4)),
            USER,
            &Visible,
            &EpochTimeFormatter,
            UserLocale::EnUs,
            &mut output,
        )
        .unwrap();
        let text = core::str::from_utf8(&output[..length]).unwrap();
        assert!(text.contains("t=31.0 | Pinned checkpoint #1 | Document | 1 objects"));
        let mut detail = [0; 512];
        let detail_len = render_checkpoint_detail(
            store.get(crate::activity::CheckpointId(1)).unwrap(),
            USER,
            &Visible,
            &EpochTimeFormatter,
            UserLocale::EnUs,
            &mut detail,
        )
        .unwrap();
        let detail = core::str::from_utf8(&detail[..detail_len]).unwrap();
        assert!(detail.contains("Object #17 at revision #3"));
        assert!(detail.contains("Pinned checkpoint"));
    }

    #[test]
    fn action_group_and_transaction_filters_are_independent() {
        let mut ledger = ActivityLedger::new();
        let group = ActionGroupId(4);
        ledger
            .append(
                ActivityDraft::new(
                    time(1),
                    USER,
                    CONTEXT,
                    crate::activity::ActionKind::ObjectChanged,
                )
                .with_action_group(group)
                .with_transaction(TransactionId(10)),
            )
            .unwrap();
        ledger
            .append(
                ActivityDraft::new(
                    time(2),
                    USER,
                    CONTEXT,
                    crate::activity::ActionKind::ObjectMoved,
                )
                .with_action_group(group)
                .with_transaction(TransactionId(11)),
            )
            .unwrap();
        let mut output = [0; 256];
        let length = render_action_group(
            &ledger,
            group,
            USER,
            &Visible,
            &EpochTimeFormatter,
            UserLocale::EnUs,
            &mut output,
        )
        .unwrap();
        let text = core::str::from_utf8(&output[..length]).unwrap();
        assert!(text.contains("Action group #4 (2 events)"));
        assert!(text.contains("Changed an item"));
        assert!(text.contains("Moved an item"));
        assert_eq!(
            ledger
                .query(ActivityQuery::default().transaction(TransactionId(10)))
                .count(),
            1
        );
        assert_eq!(
            ledger
                .query(ActivityQuery::default().action_group(group))
                .count(),
            2
        );
    }

    #[test]
    fn action_affordances_never_offer_undo_for_failed_or_irreversible_events() {
        let descriptor = UndoDescriptor::new(
            InverseKind::RestoreObjectRevision,
            RevisionId(2),
            RevisionId(1),
        );
        let mut ledger = ActivityLedger::new();
        let reversible = ledger
            .append(
                ActivityDraft::new(
                    time(1),
                    USER,
                    CONTEXT,
                    crate::activity::ActionKind::ObjectChanged,
                )
                .with_result(EventResult::Succeeded)
                .with_target(ObjectId(9))
                .unwrap()
                .with_reversibility(Reversibility::Reversible(descriptor)),
            )
            .unwrap();
        let failed = ledger
            .append(
                ActivityDraft::new(
                    time(2),
                    USER,
                    CONTEXT,
                    crate::activity::ActionKind::ObjectChanged,
                )
                .with_target(ObjectId(9))
                .unwrap()
                .with_reversibility(Reversibility::Reversible(descriptor))
                .with_result(EventResult::Failed(FailureCode::BackendFailure)),
            )
            .unwrap();
        let irreversible = ledger
            .append(
                ActivityDraft::new(
                    time(3),
                    USER,
                    CONTEXT,
                    crate::activity::ActionKind::ObjectDeleted,
                )
                .with_target(ObjectId(9))
                .unwrap()
                .with_reversibility(Reversibility::Irreversible),
            )
            .unwrap();
        assert!(event_action_availability(ledger.get(reversible).unwrap()).can_undo());
        assert!(!event_action_availability(ledger.get(failed).unwrap()).can_undo());
        assert!(!event_action_availability(ledger.get(irreversible).unwrap()).can_undo());
        let conditional = ledger
            .append(
                ActivityDraft::new(
                    time(4),
                    USER,
                    CONTEXT,
                    crate::activity::ActionKind::ObjectChanged,
                )
                .with_result(EventResult::Succeeded)
                .with_target(ObjectId(9))
                .unwrap()
                .with_reversibility(Reversibility::ConditionallyReversible(descriptor)),
            )
            .unwrap();
        assert_eq!(
            event_action_availability(ledger.get(conditional).unwrap()).undo(),
            UndoAvailability::RequiresPreconditionCheck
        );
    }

    #[test]
    fn diff_summary_is_localized_and_contains_no_content_payload() {
        let mut en = [0; 128];
        let en_len = render_diff_summary(
            ObjectId(7),
            DiffSummary::Changed {
                current_bytes: 20,
                target_bytes: 12,
            },
            UserLocale::EnUs,
            &mut en,
        )
        .unwrap();
        assert_eq!(
            &en[..en_len],
            b"Object #7: Changed (current 20 bytes, target 12 bytes)"
        );
        let mut ja = [0; 128];
        let ja_len = render_diff_summary(
            ObjectId(7),
            DiffSummary::MissingCurrent { target_bytes: 12 },
            UserLocale::JaJp,
            &mut ja,
        )
        .unwrap();
        assert_eq!(
            &ja[..ja_len],
            "オブジェクト #7: 現在のオブジェクトはありません。対象サイズ: 12 バイト".as_bytes()
        );
        assert_eq!(
            render_diff_summary(
                ObjectId(7),
                DiffSummary::Unchanged,
                UserLocale::EnUs,
                &mut [0; 3]
            ),
            Err(RenderError::BufferTooSmall)
        );
    }
}
