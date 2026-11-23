//! The submit: every check of the contract's "Every backend" column a submit can be
//! refused by, under one hold of the records with the queue's lock taken; the submit
//! stamped in the same hold; then the driver's call, with the queue's own timeline
//! signalled to the stamp after every command.

use core::any::Any;
use std::sync::Arc;

use ash::vk;
use lactose_core::Result;
use lactose_rhi_api::{QueueRole, Rule, Submit, TimelinePoint};

use crate::device::{Shared, TimelineObj, alive, lost};
use crate::encoder::VulkanEncoder;
use crate::lock;
use crate::raw::driver;
use crate::records::{Records, Release, stale};

/// The encoders of `submit` as Vulkan's, `None` for one of another backend.
fn encoders<'a>(submit: &'a Submit<'_>) -> impl Iterator<Item = Option<&'a VulkanEncoder>> {
    submit.encoders.iter().map(|encoder| {
        let any: &dyn Any = &**encoder;
        any.downcast_ref::<VulkanEncoder>()
    })
}

/// Checks every encoder of a submit to `role` against the records.
fn check(shared: &Shared, records: &Records, role: QueueRole, submit: &Submit<'_>) -> Result<()> {
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
        if let Some(error) = &encoder.failure {
            return Err(lactose_core::Error::new(error.kind(), error.message().to_owned()));
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
        for &(slot, image) in &encoder.slots {
            records.check_slot(slot.0 as usize, image)?;
        }
        for (handle, (_, touch)) in &encoder.buffers {
            let host = records.buffers.get(*handle).is_some_and(|rec| !rec.host.is_empty());
            if touch.written && host {
                return Err(
                    Rule::HostOverlap.refuse("a submit that writes a buffer during a host access")
                );
            }
        }
    }
    Ok(())
}

/// Stamps what a checked submit's encoders name with `seq` on `queue`, and marks the
/// encoders taken.
fn commit(records: &mut Records, queue: usize, seq: u64, submit: &mut Submit<'_>) {
    for encoder in submit.encoders.iter_mut() {
        let any: &mut dyn Any = &mut **encoder;
        let Some(encoder) = any.downcast_mut::<VulkanEncoder>() else { continue };
        for (handle, (_, touch)) in &encoder.buffers {
            if let Some(rec) = records.buffers.get_mut(*handle) {
                rec.stamps[queue] = seq;
                if touch.written {
                    rec.writers[queue] = seq;
                }
                if touch.written || touch.changed {
                    rec.changed[queue] = seq;
                }
            }
        }
        for handle in encoder.images.keys() {
            if let Some(rec) = records.images.get_mut(*handle) {
                rec.stamps[queue] = seq;
            }
        }
        for handle in encoder.graphics.keys() {
            if let Some(rec) = records.graphics.get_mut(*handle) {
                rec.stamps[queue] = seq;
            }
        }
        for handle in encoder.compute.keys() {
            if let Some(rec) = records.compute.get_mut(*handle) {
                rec.stamps[queue] = seq;
            }
        }
        for (slot, _) in &encoder.slots {
            records.slots[slot.0 as usize].stamps[queue] = seq;
        }
        encoder.submitted = Some((queue, seq));
    }
}

/// The semaphores of a submit's points, kept alive through the driver's call.
fn semaphores(shared: &Shared, points: &[TimelinePoint]) -> Result<Vec<(Arc<TimelineObj>, u64)>> {
    points
        .iter()
        .map(|point| {
            let object =
                shared.timelines.resolve(point.timeline).ok_or_else(|| stale("a timeline"))?;
            Ok((object, point.value))
        })
        .collect()
}

/// Submits to the queue that serves `role`.
pub(crate) fn submit(shared: &Shared, role: QueueRole, submit: Submit<'_>) -> Result<()> {
    if role == QueueRole::Present {
        alive(&shared.records())?;
        return Err(Rule::PresentQueue
            .refuse("a submit to the present role, which is reached through the swapchain alone"));
    }
    let mut submit = submit;
    let queue = shared.queue_of(role);
    let _queue = lock(&shared.queues[queue]);
    match stage(shared, queue, role, &mut submit, false)? {
        Staging::Staged(staged) => send(shared, staged, &submit, Binary::default()).0,
        Staging::Undecided(point) => {
            unreachable!("a submit is staged without a walk, yet {point:?} was left undecided")
        }
    }
}

/// A submit checked and stamped, with its points' semaphores held through the driver's call.
pub(crate) struct Staged {
    queue: usize,
    pub(crate) seq: u64,
    waits: Vec<(Arc<TimelineObj>, u64)>,
    signals: Vec<(Arc<TimelineObj>, u64)>,
}

/// What [`stage`] made of a submit.
pub(crate) enum Staging {
    Staged(Staged),
    /// A present's walk left this point undecided: nothing is stamped.
    Undecided(TimelinePoint),
}

/// The binary semaphores a present's submit adds to its batch: the acquire's to wait for,
/// where no batch has waited for it yet, and the image's present semaphore to signal.
#[derive(Clone, Copy, Default)]
pub(crate) struct Binary {
    pub(crate) wait: Option<vk::Semaphore>,
    pub(crate) signal: Option<vk::Semaphore>,
}

/// The checks of a submit to `queue` for `role`, under that queue's lock, which the caller
/// holds, and its stamp; for a present (`walk`), its release walk first, and nothing is
/// stamped where the walk leaves a point undecided.
pub(crate) fn stage(
    shared: &Shared,
    queue: usize,
    role: QueueRole,
    submit: &mut Submit<'_>,
    walk: bool,
) -> Result<Staging> {
    let read = shared.read_queues();
    let points: Vec<_> = submit
        .waits
        .iter()
        .chain(submit.signals)
        .filter_map(|point| shared.read_timeline(point.timeline))
        .collect();
    let mut records = shared.records();
    alive(&records)?;
    records.fold(&read, &points);
    check(shared, &records, role, submit)?;
    records.check_points(queue, submit.waits, submit.signals)?;
    if walk && let Release::Undecided(point) = records.release(queue, submit.waits)? {
        return Ok(Staging::Undecided(point));
    }
    let waits = semaphores(shared, submit.waits)?;
    let signals = semaphores(shared, submit.signals)?;
    let seq = records.stamp(queue, submit.waits, submit.signals);
    commit(&mut records, queue, seq, submit);
    Ok(Staging::Staged(Staged { queue, seq, waits, signals }))
}

/// The driver's call of a staged submit, under its queue's lock, which the caller holds,
/// with `binary`'s semaphores; then the hold that records it received. Where the call
/// fails and the device is not lost, an empty batch takes the stamp, waiting for
/// `binary`'s wait, so that the acquire's semaphore is spent either way, and signalling no
/// present semaphore. With the call's result, whether a batch the driver took waits for
/// `binary`'s wait: not where both calls failed.
pub(crate) fn send(
    shared: &Shared,
    staged: Staged,
    submit: &Submit<'_>,
    binary: Binary,
) -> (Result<()>, bool) {
    let Staged { queue, seq, waits, signals } = staged;
    let raw = &shared.raw;
    let target = shared.queue(queue);
    let mut ended = Ok(());
    for encoder in encoders(submit).flatten() {
        ended = ended.and_then(|()| {
            // SAFETY: the command buffer is recording, outside any pass (checked), and this
            // submit's alone: its encoder is borrowed by the submit.
            unsafe { raw.device.end_command_buffer(encoder.command_buffer) }
                .map_err(|e| driver(e, "vkEndCommandBuffer"))
        });
    }
    let buffers: Vec<_> = encoders(submit)
        .flatten()
        .map(|encoder| {
            vk::CommandBufferSubmitInfo::default().command_buffer(encoder.command_buffer)
        })
        .collect();
    let point = |semaphore: vk::Semaphore, value: u64| {
        vk::SemaphoreSubmitInfo::default()
            .semaphore(semaphore)
            .value(value)
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
    };
    let acquired: Vec<_> = binary.wait.map(|semaphore| point(semaphore, 0)).into_iter().collect();
    let wait_infos: Vec<_> = waits
        .iter()
        .map(|(object, value)| point(object.semaphore, *value))
        .chain(acquired.iter().copied())
        .collect();
    let mut signal_infos: Vec<_> = signals
        .iter()
        .map(|(object, value)| point(object.semaphore, *value))
        .chain(binary.signal.map(|semaphore| point(semaphore, 0)))
        .collect();
    signal_infos.push(point(target.timeline, seq));
    let mut spent = true;
    let submitted = ended.and_then(|()| {
        let info = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&wait_infos)
            .command_buffer_infos(&buffers)
            .signal_semaphore_infos(&signal_infos);
        // SAFETY: the queue's lock is held, which every submit to it takes; each command
        // buffer is executable and in no other pending submit (`Encoding`); every resource
        // and pipeline it names is held by its encoder and was live at the checks; every
        // signal grows past its timeline's values and pending signals (`Timeline`), and the
        // queue's own timeline grows by one. A binary wait is an acquire's semaphore, which
        // the acquire signalled and no batch has waited for; a binary signal is the image's
        // present semaphore, whose last wait, by the image's last present, is done (that
        // present's fence has signalled).
        unsafe { raw.device.queue_submit2(target.queue, &[info], vk::Fence::null()) }
            .map_err(|e| driver(e, "vkQueueSubmit2"))
    });
    if let Err(error) = &submitted {
        if error.kind() == lactose_core::ErrorKind::DeviceLost {
            shared.lost_on(vk::Result::ERROR_DEVICE_LOST);
        } else {
            // The stamp is taken: the queue's timeline reaches it with an empty batch, so
            // that the submits after it are not held back and what it named is let go.
            let own = [point(target.timeline, seq)];
            let info = vk::SubmitInfo2::default()
                .wait_semaphore_infos(&acquired)
                .signal_semaphore_infos(&own);
            // SAFETY: as above, with no command buffer and no caller's point.
            let retried =
                unsafe { raw.device.queue_submit2(target.queue, &[info], vk::Fence::null()) };
            if let Err(error) = retried {
                spent = false;
                shared.lost_on(error);
            }
        }
    }
    let mut records = shared.records();
    records.received(queue, seq, submitted.is_err());
    if records.lost {
        return (Err(lost()), false);
    }
    (submitted, spent)
}
