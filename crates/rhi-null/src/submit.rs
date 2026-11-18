//! The submit: every encoder's commands replayed, in submission order, over a copy of the
//! records they name, and the copy written back only when every check has passed, so a
//! refused submit changes nothing.

use core::any::Any;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use lactose_core::Result;
use lactose_rhi_api::{
    BufferHandle, BufferReach, BufferState, BufferTransition, ComputePipelineHandle,
    GraphicsPipelineHandle, ImageHandle, ImageState, ImageTransition, QueueLayout, QueueRole, Rule,
    Submit,
};

use crate::device::{Shared, exclusive};
use crate::encoder::{Attach, Cmd, NullEncoder, SlotDecl};
use crate::records::{
    BufferRec, CopyBytes, ImageRec, Records, Stamps, Sub, SubmitRec, Transfer, queue_index, stale,
};

/// What a submit that passed its checks changes.
pub(crate) struct Plan {
    queue: usize,
    images: HashMap<ImageHandle, ImageRec>,
    buffers: HashMap<BufferHandle, BufferRec>,
    /// The buffers the submit writes.
    written: HashSet<BufferHandle>,
    slots: BTreeSet<usize>,
    graphics: HashSet<GraphicsPipelineHandle>,
    compute: HashSet<ComputePipelineHandle>,
    copies: Vec<CopyBytes>,
}

/// The encoders of `submit` as null's, or the refusal of one that is not.
fn encoders<'a>(submit: &'a Submit<'_>) -> impl Iterator<Item = Option<&'a NullEncoder>> {
    submit.encoders.iter().map(|encoder| {
        let any: &dyn Any = &**encoder;
        any.downcast_ref::<NullEncoder>()
    })
}

/// Checks a submit to `role`, under the records as they are: every encoder, then every
/// command in order, then its points. `present` is the image a present shows, which the
/// submit must leave in `Present`.
pub(crate) fn check(
    shared: &Shared,
    records: &Records,
    role: QueueRole,
    submit: &Submit<'_>,
    present: Option<ImageHandle>,
) -> Result<Plan> {
    let queue = queue_index(shared.caps.queues.serving(role));
    for encoder in encoders(submit) {
        let Some(encoder) = encoder else {
            return Err(Rule::Encoding.refuse("an encoder of another backend"));
        };
        if !core::ptr::eq(Arc::as_ptr(&encoder.shared), shared) {
            return Err(Rule::Encoding.refuse("an encoder of another device"));
        }
        if encoder.role != role {
            return Err(Rule::Encoding.refuse(format!(
                "an encoder of the {:?} role in a submit to {role:?}",
                encoder.role
            )));
        }
        if encoder.submitted.is_some() {
            return Err(Rule::Encoding.refuse("an encoder submitted again with no reset"));
        }
        if let Some((rule, detail)) = &encoder.breach {
            return Err(rule.refuse(detail.clone()));
        }
        if encoder.open() {
            return Err(Rule::Encoding.refuse("an encoder with a pass left open"));
        }
        let stale_handle = encoder.buffers.keys().any(|h| records.buffers.get(*h).is_none())
            || encoder.images.keys().any(|h| records.images.get(*h).is_none())
            || encoder.graphics.keys().any(|h| records.graphics.get(*h).is_none())
            || encoder.compute.keys().any(|h| records.compute.get(*h).is_none());
        if stale_handle {
            return Err(Rule::Destroyed.refuse("a handle destroyed after it was recorded"));
        }
    }
    let mut replay = Replay {
        records,
        layout: shared.caps.queues,
        queue,
        cover: records.cover(queue, submit.waits),
        coherent: shared.caps.coherent_host_memory,
        plan: Plan {
            queue,
            images: HashMap::new(),
            buffers: HashMap::new(),
            written: HashSet::new(),
            slots: BTreeSet::new(),
            graphics: HashSet::new(),
            compute: HashSet::new(),
            copies: Vec::new(),
        },
        pass: None,
    };
    for encoder in encoders(submit).flatten() {
        for command in &encoder.commands {
            replay.command(encoder, command)?;
        }
    }
    if let Some(image) = present {
        let rec = replay.image(image)?;
        if rec.subs.iter().any(|sub| sub.state != ImageState::Present) {
            return Err(
                Rule::State.refuse("a present of an image its submit leaves in another state")
            );
        }
    }
    records.check_points(submit.waits, submit.signals, &replay.cover)?;
    Ok(replay.plan)
}

/// Makes a checked submit's changes, under the same hold as its check: its stamps, the
/// records it changed, its encoders pending, and the submit in its queue's list. Runs
/// whatever it lets run. Returns its sequence number.
pub(crate) fn commit(records: &mut Records, plan: Plan, submit: &mut Submit<'_>) -> u64 {
    let queue = plan.queue;
    let seq = records.queues[queue].next;
    records.queues[queue].next += 1;
    for (handle, mut rec) in plan.images {
        rec.stamps[queue] = seq;
        if let Some(slot) = records.images.get_mut(handle) {
            *slot = rec;
        }
    }
    for (handle, mut rec) in plan.buffers {
        rec.stamps[queue] = seq;
        if plan.written.contains(&handle) {
            rec.writers[queue] = seq;
        }
        if let Some(slot) = records.buffers.get_mut(handle) {
            *slot = rec;
        }
    }
    for slot in plan.slots {
        records.slots[slot].stamps[queue] = seq;
    }
    for pipeline in plan.graphics {
        if let Some(rec) = records.graphics.get_mut(pipeline) {
            rec.stamps[queue] = seq;
        }
    }
    for pipeline in plan.compute {
        if let Some(rec) = records.compute.get_mut(pipeline) {
            rec.stamps[queue] = seq;
        }
    }
    for encoder in submit.encoders.iter_mut() {
        let any: &mut dyn Any = &mut **encoder;
        if let Some(encoder) = any.downcast_mut::<NullEncoder>() {
            encoder.submitted = Some((queue, seq));
        }
    }
    records.queues[queue].submits.push_back(SubmitRec {
        seq,
        waits: submit.waits.to_vec(),
        signals: submit.signals.to_vec(),
        copies: plan.copies,
    });
    records.run();
    records.settle_presents();
    seq
}

/// The pass being replayed.
struct Pass {
    attachments: Vec<Attach>,
    slots: Vec<SlotDecl>,
    buffers: Vec<BufferReach>,
}

struct Replay<'a> {
    records: &'a Records,
    layout: QueueLayout,
    queue: usize,
    cover: Stamps,
    coherent: bool,
    plan: Plan,
    pass: Option<Pass>,
}

impl Replay<'_> {
    /// The image's record as the submit has changed it so far.
    fn image(&mut self, handle: ImageHandle) -> Result<&mut ImageRec> {
        if !self.plan.images.contains_key(&handle) {
            let rec = self.records.images.get(handle).ok_or_else(|| stale("an image"))?;
            self.plan.images.insert(handle, rec.clone());
        }
        Ok(self.plan.images.get_mut(&handle).expect("inserted above"))
    }

    /// The image's record as the submit has changed it so far, without taking it in.
    fn peek(&self, handle: ImageHandle) -> Option<&ImageRec> {
        self.plan.images.get(&handle).or_else(|| self.records.images.get(handle))
    }

    fn buffer(&mut self, handle: BufferHandle) -> Result<&mut BufferRec> {
        if !self.plan.buffers.contains_key(&handle) {
            let rec = self.records.buffers.get(handle).ok_or_else(|| stale("a buffer"))?;
            self.plan.buffers.insert(handle, rec.clone());
        }
        Ok(self.plan.buffers.get_mut(&handle).expect("inserted above"))
    }

    fn command(&mut self, encoder: &NullEncoder, command: &Cmd) -> Result<()> {
        match command {
            Cmd::Images(transitions) => {
                for transition in transitions {
                    self.image_transition(transition)?;
                }
            }
            Cmd::Buffers(transitions) => {
                for transition in transitions {
                    self.buffer_transition(transition)?;
                }
            }
            Cmd::Alias(image) => self.alias(*image)?,
            Cmd::CopyBuffer { source, destination, regions } => {
                self.use_buffer(*source, BufferState::CopySource, true)?;
                self.use_buffer(*destination, BufferState::CopyDestination, false)?;
                let ranges: Vec<_> = regions
                    .iter()
                    .map(|region| {
                        (region.destination_offset, region.destination_offset + region.size)
                    })
                    .collect();
                self.wrote_buffer(*destination, &ranges)?;
                self.plan.copies.push(CopyBytes {
                    source: Arc::clone(&encoder.buffers[source]),
                    destination: Arc::clone(&encoder.buffers[destination]),
                    regions: regions.clone(),
                });
            }
            Cmd::CopyToImage { source, destination, subresources } => {
                self.use_buffer(*source, BufferState::CopySource, true)?;
                for &(mip, layer) in subresources {
                    self.use_image(
                        *destination,
                        mip,
                        layer,
                        ImageState::CopyDestination,
                        false,
                        Rule::State,
                    )?;
                }
                let rec = self.image(*destination)?;
                for &(mip, layer) in subresources {
                    let at = rec.sub(mip, layer);
                    rec.subs[at].written = true;
                    rec.subs[at].defined = true;
                }
            }
            Cmd::CopyToBuffer { source, subresources, destination, ranges } => {
                for &(mip, layer) in subresources {
                    self.use_image(*source, mip, layer, ImageState::CopySource, true, Rule::State)?;
                }
                self.use_buffer(*destination, BufferState::CopyDestination, false)?;
                self.wrote_buffer(*destination, ranges)?;
            }
            Cmd::BeginRender { colour, depth, slots, buffers } => {
                self.declare(slots, buffers)?;
                let attachments: Vec<Attach> = colour.iter().chain(depth).copied().collect();
                for attach in &attachments {
                    let target = attach.target;
                    self.use_image(
                        target.image,
                        target.mip,
                        target.layer,
                        attach.state,
                        attach.keep,
                        Rule::State,
                    )?;
                }
                self.pass =
                    Some(Pass { attachments, slots: slots.clone(), buffers: buffers.clone() });
            }
            Cmd::BeginCompute { slots, buffers } => {
                self.declare(slots, buffers)?;
                self.pass = Some(Pass {
                    attachments: Vec::new(),
                    slots: slots.clone(),
                    buffers: buffers.clone(),
                });
            }
            Cmd::EndPass => self.end_pass()?,
            Cmd::Graphics(pipeline) => {
                self.plan.graphics.insert(*pipeline);
            }
            Cmd::Compute(pipeline) => {
                self.plan.compute.insert(*pipeline);
            }
            Cmd::Draw { index, indirect } => {
                self.read_slots()?;
                if let Some(index) = index {
                    self.use_buffer(*index, BufferState::Index, true)?;
                }
                for buffer in indirect {
                    self.use_buffer(*buffer, BufferState::Indirect, true)?;
                }
            }
            Cmd::Dispatch { indirect } => {
                self.read_slots()?;
                if let Some(buffer) = indirect {
                    self.use_buffer(*buffer, BufferState::Indirect, true)?;
                }
            }
        }
        Ok(())
    }

    /// A pass's declarations as it begins: each slot unmarked and holding the image the
    /// pass resolved, each buffer in its declared state.
    fn declare(&mut self, slots: &[SlotDecl], buffers: &[BufferReach]) -> Result<()> {
        for &(slot, image) in slots {
            let at = slot.0 as usize;
            let rec = &self.records.slots[at];
            if rec.mark {
                return Err(Rule::HeapWrite.refuse(format!(
                    "a pass that declares slot {}, whose write is in progress",
                    slot.0
                )));
            }
            if rec.image != image {
                return Err(Rule::HeapLayout.refuse(format!(
                    "a pass that declares slot {}, written since the pass began",
                    slot.0
                )));
            }
            self.plan.slots.insert(at);
        }
        for reach in buffers {
            let write = reach.state == BufferState::ShaderWrite;
            self.use_buffer(reach.buffer, reach.state, !write)?;
            if write && !self.buffer(reach.buffer)?.host.is_empty() {
                return Err(
                    Rule::HostOverlap.refuse("a submit that writes a buffer during a host access")
                );
            }
        }
        Ok(())
    }

    /// A draw's or dispatch's read of every declared slot's image.
    fn read_slots(&mut self) -> Result<()> {
        let slots = self.pass.as_ref().map(|pass| pass.slots.clone()).unwrap_or_default();
        for (slot, image) in slots {
            let Some(image) = image else {
                return Err(Rule::HeapLayout.refuse(format!("slot {} declared and empty", slot.0)));
            };
            let rec = self.image(image)?;
            let (mips, layers) =
                (rec.mips, u32::try_from(rec.subs.len()).unwrap_or(u32::MAX) / rec.mips);
            for layer in 0..layers {
                for mip in 0..mips {
                    self.use_image(
                        image,
                        mip,
                        layer,
                        ImageState::SampledRead,
                        true,
                        Rule::HeapLayout,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn end_pass(&mut self) -> Result<()> {
        let Some(pass) = self.pass.take() else { return Ok(()) };
        for attach in pass.attachments {
            let target = attach.target;
            let rec = self.image(target.image)?;
            let at = rec.sub(target.mip, target.layer);
            rec.subs[at].written = true;
            rec.subs[at].defined = attach.store;
        }
        for reach in pass.buffers {
            if reach.state == BufferState::ShaderWrite {
                let size = self.buffer(reach.buffer)?.size;
                self.wrote_buffer(reach.buffer, &[(0, size)])?;
            }
        }
        Ok(())
    }

    /// Notes a device write of `ranges` of a buffer, which no host access may overlap.
    fn wrote_buffer(&mut self, handle: BufferHandle, ranges: &[(u64, u64)]) -> Result<()> {
        let coherent = self.coherent;
        let rec = self.buffer(handle)?;
        if !rec.host.is_empty() {
            return Err(
                Rule::HostOverlap.refuse("a submit that writes a buffer during a host access")
            );
        }
        rec.sub.written = true;
        rec.sub.defined = true;
        if !coherent {
            for &(start, end) in ranges {
                rec.uninvalidated.add(start, end);
            }
        }
        self.plan.written.insert(handle);
        Ok(())
    }

    /// Refuses a change of a resource's state or owner while a pending submit of another
    /// queue uses it, with no wait for that submit.
    fn covered(&self, stamps: &Stamps) -> Result<()> {
        for (other, (stamp, cover)) in stamps.iter().zip(&self.cover).enumerate() {
            if other != self.queue && stamp > cover {
                return Err(Rule::Queue.refuse(
                    "a change of state or owner while a pending submit of another queue uses the \
                     resource, with no wait for it",
                ));
            }
        }
        Ok(())
    }

    /// A placed image's use: begun by an alias call, or its first use at a placement no
    /// image that overlaps it holds.
    fn touch_placed(&mut self, handle: ImageHandle) -> Result<()> {
        let queue = self.queue;
        let rec = self.image(handle)?;
        let Some(placed) = rec.placed else { return Ok(()) };
        if !rec.active {
            let held = self
                .overlapping(handle, placed)
                .into_iter()
                .any(|other| self.peek(other).is_some_and(|rec| rec.active));
            if held {
                return Err(Rule::Alias.refuse(
                    "a placed image used after another image used its memory, with no alias call since",
                ));
            }
        }
        let rec = self.image(handle)?;
        rec.active = true;
        rec.used_on = Some(queue);
        Ok(())
    }

    /// The images other than `handle` placed over its bytes.
    fn overlapping(&self, handle: ImageHandle, placed: crate::records::Placed) -> Vec<ImageHandle> {
        let Some(block) = self.records.blocks.get(placed.block) else { return Vec::new() };
        block
            .images
            .iter()
            .copied()
            .filter(|&other| other != handle)
            .filter(|&other| {
                self.peek(other)
                    .and_then(|rec| rec.placed)
                    .is_some_and(|theirs| theirs.start < placed.end && placed.start < theirs.end)
            })
            .collect()
    }

    fn alias(&mut self, handle: ImageHandle) -> Result<()> {
        let queue = self.queue;
        let Some(placed) = self.image(handle)?.placed else {
            return Err(Rule::Usage.refuse("an alias call of an image not placed in a block"));
        };
        let others = self.overlapping(handle, placed);
        for &other in &others {
            if self.peek(other).and_then(|rec| rec.used_on).is_some_and(|used| used != queue) {
                return Err(Rule::Queue.refuse(
                    "an alias call on another queue than the one the images it ends were used on",
                ));
            }
        }
        for other in others {
            if self.peek(other).is_some_and(|rec| rec.active) {
                self.image(other)?.active = false;
            }
        }
        let owner = exclusive(self.image(handle)?.sharing).then_some(queue);
        let rec = self.image(handle)?;
        rec.active = true;
        rec.used_on = Some(queue);
        for sub in &mut rec.subs {
            *sub = Sub { owner, ..Sub::new(ImageState::Undefined) };
        }
        Ok(())
    }

    /// A use of one subresource in `need`, a read of its contents where `read`.
    fn use_image(
        &mut self,
        handle: ImageHandle,
        mip: u32,
        layer: u32,
        need: ImageState,
        read: bool,
        rule: Rule,
    ) -> Result<()> {
        self.touch_placed(handle)?;
        let queue = self.queue;
        let rec = self.image(handle)?;
        if rec.acquired == Some(false) {
            return Err(Rule::Swapchain
                .refuse("a swapchain image used outside its acquire and its present"));
        }
        let owned = exclusive(rec.sharing);
        let sub = rec.subs[rec.sub(mip, layer)];
        use_sub(&sub, owned, queue, need, read, rule)
    }

    fn use_buffer(&mut self, handle: BufferHandle, need: BufferState, read: bool) -> Result<()> {
        let queue = self.queue;
        let rec = self.buffer(handle)?;
        let owned = exclusive(rec.sharing);
        use_sub(&rec.sub, owned, queue, need, read, Rule::State)?;
        if !rec.unflushed.is_empty() {
            return Err(Rule::Unflushed
                .refuse("a submit that reads a buffer with a written range not flushed"));
        }
        Ok(())
    }

    fn image_transition(&mut self, transition: &ImageTransition) -> Result<()> {
        let layout = self.layout_queues(transition.from.queue, transition.to.queue);
        self.touch_placed(transition.image)?;
        let queue = self.queue;
        let stamps = self.image(transition.image)?.stamps;
        self.covered(&stamps)?;
        let rec = self.image(transition.image)?;
        if rec.acquired == Some(false) {
            return Err(Rule::Swapchain
                .refuse("a swapchain image used outside its acquire and its present"));
        }
        let owned = exclusive(rec.sharing);
        let range = transition.range;
        for layer in range.layer..range.layer + range.layers {
            for mip in range.mip..range.mip + range.mips {
                let at = rec.sub(mip, layer);
                let next = change(
                    rec.subs[at],
                    owned,
                    queue,
                    layout,
                    transition.from.state,
                    transition.to.state,
                    ImageState::Undefined,
                    true,
                )?;
                rec.subs[at] = next;
            }
        }
        Ok(())
    }

    fn buffer_transition(&mut self, transition: &BufferTransition) -> Result<()> {
        let layout = self.layout_queues(transition.from.queue, transition.to.queue);
        let queue = self.queue;
        let stamps = self.buffer(transition.buffer)?.stamps;
        self.covered(&stamps)?;
        let rec = self.buffer(transition.buffer)?;
        let owned = exclusive(rec.sharing);
        rec.sub = change(
            rec.sub,
            owned,
            queue,
            layout,
            transition.from.state,
            transition.to.state,
            BufferState::Undefined,
            false,
        )?;
        Ok(())
    }

    /// The queues a transition's two roles resolve to.
    fn layout_queues(&self, from: QueueRole, to: QueueRole) -> (usize, usize) {
        (queue_index(self.layout.serving(from)), queue_index(self.layout.serving(to)))
    }
}

/// A use of one subresource, or of a buffer.
fn use_sub<S: Copy + PartialEq + core::fmt::Debug>(
    sub: &Sub<S>,
    owned: bool,
    queue: usize,
    need: S,
    read: bool,
    rule: Rule,
) -> Result<()> {
    if sub.transfer.is_some() {
        return Err(Rule::Queue.refuse("a use of a resource between its release and its acquire"));
    }
    if sub.state != need {
        return Err(
            rule.refuse(format!("a use that needs {need:?}, of a resource in {:?}", sub.state))
        );
    }
    if owned && sub.owner != Some(queue) {
        return Err(Rule::Queue.refuse("a use on a queue that does not own the resource"));
    }
    if sub.written {
        return Err(Rule::State.refuse(
            "a use of a resource an earlier pass or copy wrote, with no transition since",
        ));
    }
    if read && !sub.defined {
        return Err(Rule::UndefinedRead.refuse("a read of contents that are undefined"));
    }
    Ok(())
}

/// One transition of one subresource, or of a buffer, recorded on `queue`: the
/// subresource after it, or the refusal. An image's contents are discarded by a
/// transition from undefined; a buffer's only when it moves to another queue
/// (`discards` is false for a buffer).
#[expect(clippy::too_many_arguments, reason = "one transition's every input, used once")]
fn change<S: Copy + PartialEq + core::fmt::Debug>(
    sub: Sub<S>,
    owned: bool,
    queue: usize,
    (from_queue, to_queue): (usize, usize),
    from: S,
    to: S,
    undefined: S,
    discards: bool,
) -> Result<Sub<S>> {
    if from == undefined {
        if owned && to_queue != queue {
            return Err(Rule::Queue.refuse(
                "a transition from undefined recorded on another queue than the one it takes the \
                 resource for",
            ));
        }
        let moved = owned && sub.owner.is_some_and(|owner| owner != to_queue);
        return Ok(Sub {
            state: to,
            owner: owned.then_some(to_queue),
            transfer: None,
            defined: if discards || moved { false } else { sub.defined },
            written: false,
        });
    }
    if !owned {
        if sub.state != from {
            return Err(Rule::State
                .refuse(format!("a transition from {from:?} of a resource in {:?}", sub.state)));
        }
        return Ok(Sub { state: to, written: false, ..sub });
    }
    if from_queue == to_queue {
        if sub.state != from {
            return Err(Rule::State
                .refuse(format!("a transition from {from:?} of a resource in {:?}", sub.state)));
        }
        if sub.owner != Some(from_queue) || queue != from_queue {
            return Err(
                Rule::Queue.refuse("a transition on a queue that does not own the resource")
            );
        }
        if sub.transfer.is_some() {
            return Err(Rule::Queue.refuse("a transition between a release and its acquire"));
        }
        return Ok(Sub { state: to, written: false, ..sub });
    }
    let transfer = Transfer { from, from_queue, to, to_queue };
    if queue == from_queue {
        if sub.state != from {
            return Err(Rule::State
                .refuse(format!("a release from {from:?} of a resource in {:?}", sub.state)));
        }
        if sub.owner != Some(from_queue) || sub.transfer.is_some() {
            return Err(Rule::Queue.refuse("a release by a queue that does not own the resource"));
        }
        return Ok(Sub { transfer: Some(transfer), ..sub });
    }
    if queue == to_queue {
        if sub.transfer != Some(transfer) {
            return Err(Rule::Queue.refuse("an acquire with no equal release submitted"));
        }
        return Ok(Sub { state: to, owner: Some(to_queue), transfer: None, written: false, ..sub });
    }
    Err(Rule::Queue.refuse("a transfer recorded on a queue that neither releases nor acquires"))
}
