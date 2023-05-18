//! Provides the [`SubmitBatch`] struct to batch submits together and synchronize between them easily.

use std::sync::Arc;

use anyhow::{ensure, Result};
use ash::vk;

use crate::{Allocator, CmdBuffer, Device, ExecutionManager, Fence, InFlightContext, PipelineStage, Semaphore};
use crate::command_buffer::CommandBuffer;
use crate::sync::domain::ExecutionDomain;

#[derive(Debug)]
struct SubmitInfo<D: ExecutionDomain> {
    cmd: CommandBuffer<D>,
    signal_semaphore: Option<Arc<Semaphore>>,
    wait_semaphores: Vec<Arc<Semaphore>>,
    wait_stages: Vec<PipelineStage>,
}

/// A handle to a submit inside a batch.
/// Can be used to make submits wait on other submits inside a single batch
#[derive(Debug, Copy, Clone)]
pub struct SubmitHandle {
    index: usize,
}

/// A batch of submits containing multiple command buffers that possibly
/// wait on each other using semaphores. An example usage is given in the documentation for
/// [`ExecutionManager::start_submit_batch`].
#[derive(Debug)]
pub struct SubmitBatch<D: ExecutionDomain> {
    device: Device,
    exec: ExecutionManager,
    submits: Vec<SubmitInfo<D>>,
    signal_fence: Fence,
}

impl<D: ExecutionDomain + 'static> SubmitBatch<D> {
    pub(crate) fn new(device: Device, exec: ExecutionManager) -> Result<Self> {
        Ok(Self {
            submits: vec![],
            signal_fence: Fence::new(device.clone(), false)?,
            device,
            exec,
        })
    }

    fn get_submit_semaphore(&self, submit: SubmitHandle) -> Option<Arc<Semaphore>> {
        self.submits
            .get(submit.index)
            .and_then(|submit| submit.signal_semaphore.clone())
    }

    fn submit_after(&mut self, handles: &[SubmitHandle], cmd: CommandBuffer<D>, wait_stages: &[PipelineStage]) -> Result<SubmitHandle> {
        let wait_semaphores = handles
            .iter()
            .map(|handle| self.get_submit_semaphore(*handle).unwrap())
            .collect::<Vec<_>>();

        self.submits.push(SubmitInfo {
            cmd,
            signal_semaphore: Some(Arc::new(Semaphore::new(self.device.clone())?)),
            wait_semaphores,
            wait_stages: wait_stages.to_vec(),
        });

        Ok(SubmitHandle {
            index: self.submits.len() - 1,
        })
    }

    /// Must be used to submit the final command buffer in the frame
    pub fn submit_for_present<A: Allocator>(&mut self, cmd: CommandBuffer<D>, ifc: &InFlightContext<A>) -> Result<SubmitHandle> {
        self.submit_for_present_after(cmd, ifc, &[], &[])
    }

    /// Submit the frame commands, waiting on the given previous submits
    pub fn submit_for_present_after<A: Allocator>(
        &mut self,
        cmd: CommandBuffer<D>,
        ifc: &InFlightContext<A>,
        submits: &[SubmitHandle],
        wait_stages: &[PipelineStage],
    ) -> Result<SubmitHandle> {
        ensure!(
            submits.len() == wait_stages.len(),
            "Number of wait stages must match number of submits"
        );
        let mut wait_semaphores = submits
            .iter()
            .map(|handle| self.get_submit_semaphore(*handle).unwrap())
            .collect::<Vec<_>>();
        let mut wait_stages = wait_stages.to_vec();
        let frame_wait_semaphore = ifc.wait_semaphore.clone().expect("cannot submit for present outside of a frame context");
        // Add this semaphore as a wait semaphore for the first submit, or to the frame commands if there is no other submit
        match self.submits.first_mut() {
            None => {
                wait_semaphores.push(frame_wait_semaphore);
                wait_stages.push(PipelineStage::COLOR_ATTACHMENT_OUTPUT);
            }
            Some(submit) => {
                submit.wait_stages.push(PipelineStage::TOP_OF_PIPE);
                submit.wait_semaphores.push(frame_wait_semaphore);
            }
        }

        self.submits.push(SubmitInfo {
            cmd,
            signal_semaphore: Some(
                ifc.signal_semaphore
                    .clone()
                    .expect("cannot submit for present outside of a frame context"),
            ),
            wait_semaphores,
            wait_stages,
        });

        Ok(SubmitHandle {
            index: self.submits.len() - 1,
        })
    }

    /// Submit the frame commands, waiting on all previous submissions in the same pipeline stage
    pub fn submit_for_present_after_all<A: Allocator>(
        &mut self,
        cmd: CommandBuffer<D>,
        ifc: &InFlightContext<A>,
        wait_stage: PipelineStage,
    ) -> Result<SubmitHandle> {
        let submits = (0..self.submits.len())
            .map(|index| SubmitHandle {
                index,
            })
            .collect::<Vec<_>>();
        let stages = vec![wait_stage; self.submits.len()];
        self.submit_for_present_after(cmd, ifc, &submits, &stages)
    }

    /// Submit a new command buffer in this batch with no dependencies.
    pub fn submit(&mut self, cmd: CommandBuffer<D>) -> Result<SubmitHandle> {
        self.submits.push(SubmitInfo {
            cmd,
            signal_semaphore: Some(Arc::new(Semaphore::new(self.device.clone())?)),
            wait_semaphores: vec![],
            wait_stages: vec![],
        });

        Ok(SubmitHandle {
            index: self.submits.len() - 1,
        })
    }

    /// Finish this batch by submitting it to the execution manager.
    /// This returns a [`Fence`] that can be awaited to wait for completion.
    pub fn finish(self) -> Result<Fence> {
        struct PerSubmit {
            wait_semaphores: Vec<vk::SemaphoreSubmitInfo>,
            cmd_buffer: Vec<vk::CommandBufferSubmitInfo>,
            signal_semaphores: Vec<vk::SemaphoreSubmitInfo>,
        }

        let mut per_submit_info = Vec::new();
        for submit in &self.submits {
            let info = PerSubmit {
                wait_semaphores: submit
                    .wait_semaphores
                    .iter()
                    .zip(&submit.wait_stages)
                    .map(|(semaphore, stage)| vk::SemaphoreSubmitInfo {
                        s_type: vk::StructureType::SEMAPHORE_SUBMIT_INFO,
                        p_next: std::ptr::null(),
                        semaphore: unsafe { semaphore.handle() },
                        value: 0,
                        stage_mask: *stage,
                        device_index: 0,
                    })
                    .collect(),
                cmd_buffer: vec![vk::CommandBufferSubmitInfo {
                    s_type: vk::StructureType::COMMAND_BUFFER_SUBMIT_INFO,
                    p_next: std::ptr::null(),
                    command_buffer: unsafe { submit.cmd.handle() },
                    device_mask: 0,
                }],
                signal_semaphores: match &submit.signal_semaphore {
                    None => {
                        vec![]
                    }
                    Some(semaphore) => {
                        vec![vk::SemaphoreSubmitInfo {
                            s_type: vk::StructureType::SEMAPHORE_SUBMIT_INFO,
                            p_next: std::ptr::null(),
                            semaphore: unsafe { semaphore.handle() },
                            value: 0,
                            stage_mask: PipelineStage::BOTTOM_OF_PIPE,
                            device_index: 0,
                        }]
                    }
                },
            };
            per_submit_info.push(info);
        }
        let submits = per_submit_info
            .iter()
            .map(|submit| vk::SubmitInfo2 {
                s_type: vk::StructureType::SUBMIT_INFO_2,
                p_next: std::ptr::null(),
                flags: Default::default(),
                wait_semaphore_info_count: submit.wait_semaphores.len() as u32,
                p_wait_semaphore_infos: submit.wait_semaphores.as_ptr(),
                command_buffer_info_count: submit.cmd_buffer.len() as u32,
                p_command_buffer_infos: submit.cmd_buffer.as_ptr(),
                signal_semaphore_info_count: submit.signal_semaphores.len() as u32,
                p_signal_semaphore_infos: submit.signal_semaphores.as_ptr(),
            })
            .collect::<Vec<_>>();

        self.exec.submit_batch::<D>(submits.as_slice(), &self.signal_fence)?;
        let fence = self.signal_fence.with_cleanup(move || {
            // Take ownership of every resource inside the submit batch, to delete it afterwards
            for mut submit in self.submits {
                unsafe {
                    submit.cmd.delete(self.exec.clone()).unwrap();
                }
            }
        });

        Ok(fence)
    }
}

impl SubmitHandle {
    /// Add another submit to the batch that waits on this submit at the specified wait stage mask.
    pub fn then<D: ExecutionDomain + 'static>(&self, wait_stage: PipelineStage, cmd: CommandBuffer<D>, batch: &mut SubmitBatch<D>) -> Result<SubmitHandle> {
        batch.submit_after(std::slice::from_ref(self), cmd, std::slice::from_ref(&wait_stage))
    }
}
