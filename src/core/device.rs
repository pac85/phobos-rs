use std::collections::HashSet;
use std::ffi::{CStr, CString, NulError};
use std::fmt::Formatter;
use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use ash::vk;

use crate::{AppSettings, PhysicalDevice, VkInstance, WindowInterface};
use crate::util::string::unwrap_to_raw_strings;

/// Device extensions that phobos requests but might not be available.
/// # Example
/// ```
/// # use phobos::*;
/// # use phobos::core::device::ExtensionID;
///
/// fn has_dynamic_state(device: Device) -> bool {
///     device.is_extension_enabled(ExtensionID::ExtendedDynamicState3)
/// }
/// ```
#[derive(Debug, Eq, PartialEq, Hash)]
pub enum ExtensionID {
    /// `VK_EXT_extended_dynamic_state3` provides more dynamic states to pipeline objects.
    ExtendedDynamicState3,
}

impl std::fmt::Display for ExtensionID {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Derivative)]
#[derivative(Debug)]
struct DeviceInner {
    #[derivative(Debug = "ignore")]
    handle: ash::Device,
    queue_families: Vec<u32>,
    properties: vk::PhysicalDeviceProperties,
    extensions: HashSet<ExtensionID>,
    #[derivative(Debug = "ignore")]
    dynamic_state3: Option<ash::extensions::ext::ExtendedDynamicState3>,
}

/// Wrapper around a `VkDevice`. The device provides access to almost the entire
/// Vulkan API. Internal state is wrapped in an `Arc<DeviceInner>`, so this is safe
/// to clone
#[derive(Debug, Clone)]
pub struct Device {
    inner: Arc<DeviceInner>,
}

fn add_if_supported(
    ext: ExtensionID,
    name: &CStr,
    enabled_set: &mut HashSet<ExtensionID>,
    names: &mut Vec<CString>,
    extensions: &[vk::ExtensionProperties],
) -> bool {
    // First check if extension is supported
    if extensions
        .iter()
        // SAFETY: This pointer is obtained from a c string that was returned from a Vulkan API call. We can assume the
        // Vulkan API always returns valid strings.
        .any(|ext| name == unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) })
    {
        enabled_set.insert(ext);
        names.push(CString::from(name));
        true
    } else {
        info!(
            "Requested extension {} is not available. Some features might be missing.",
            name.to_bytes().escape_ascii()
        );
        false
    }
}

impl Device {
    /// Create a new Vulkan device. This is the main interface point with the Vulkan API.
    /// # Errors
    /// * Can fail if vulkan device init fails. This is possible if an optional feature was enabled that is not supported.
    pub fn new<Window: WindowInterface>(instance: &VkInstance, physical_device: &PhysicalDevice, settings: &AppSettings<Window>) -> Result<Self> {
        let mut priorities = Vec::<f32>::new();
        let queue_create_infos = physical_device
            .queue_families()
            .iter()
            .enumerate()
            .flat_map(|(index, family_info)| {
                let count = physical_device
                    .queues()
                    .iter()
                    .filter(|queue| queue.family_index == index as u32)
                    .count();
                if count == 0 {
                    return None;
                }
                let count = count.min(family_info.queue_count as usize);
                priorities.resize(usize::max(priorities.len(), count), 1.0);
                Some(vk::DeviceQueueCreateInfo {
                    queue_family_index: index as u32,
                    queue_count: count as u32,
                    p_queue_priorities: priorities.as_ptr(),
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>();
        let mut extension_names: Vec<CString> = settings
            .gpu_requirements
            .device_extensions
            .iter()
            .map(|ext| CString::new(ext.clone()))
            .collect::<Result<Vec<CString>, NulError>>()?;

        // SAFETY: Vulkan API call. We have a valid reference to a PhysicalDevice, so handle() is valid.
        let available_extensions = unsafe { instance.enumerate_device_extension_properties(physical_device.handle())? };
        let mut enabled_extensions = HashSet::new();
        // Add the extensions we want, but that are not required.
        let dynamic_state3_supported = add_if_supported(
            ExtensionID::ExtendedDynamicState3,
            ash::extensions::ext::ExtendedDynamicState3::name(),
            &mut enabled_extensions,
            &mut extension_names,
            available_extensions.as_slice(),
        );

        // Add required extensions
        if settings.window.is_some() {
            extension_names.push(CString::from(ash::extensions::khr::Swapchain::name()));
        }

        info!("Enabled device extensions:");
        for ext in &extension_names {
            info!("{:?}", ext);
        }

        let mut features_1_1 = settings.gpu_requirements.features_1_1;
        let mut features_1_2 = settings.gpu_requirements.features_1_2;
        let mut features_1_3 = settings.gpu_requirements.features_1_3;
        features_1_3.synchronization2 = vk::TRUE;
        features_1_3.dynamic_rendering = vk::TRUE;
        features_1_3.maintenance4 = vk::TRUE;

        let extension_names_raw = unwrap_to_raw_strings(extension_names.as_slice());
        let mut info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(queue_create_infos.as_slice())
            .enabled_extension_names(extension_names_raw.as_slice())
            .enabled_features(&settings.gpu_requirements.features)
            .push_next(&mut features_1_1)
            .push_next(&mut features_1_2)
            .push_next(&mut features_1_3);

        let mut features_dynamic_state3 = vk::PhysicalDeviceExtendedDynamicState3FeaturesEXT {
            extended_dynamic_state3_polygon_mode: vk::TRUE,
            ..Default::default()
        };
        if dynamic_state3_supported {
            info = info.push_next(&mut features_dynamic_state3);
        }
        let info = info.build();

        let handle = unsafe { instance.create_device(physical_device.handle(), &info, None)? };
        #[cfg(feature = "log-objects")]
        trace!("Created new VkDevice {:p}", handle.handle());

        let dynamic_state3 = if dynamic_state3_supported {
            Some(ash::extensions::ext::ExtendedDynamicState3::new(instance, &handle))
        } else {
            None
        };

        let inner = DeviceInner {
            handle,
            queue_families: queue_create_infos.iter().map(|info| info.queue_family_index).collect(),
            properties: *physical_device.properties(),
            extensions: enabled_extensions,
            dynamic_state3,
        };

        Ok(Device {
            inner: Arc::new(inner),
        })
    }

    /// Wait for the device to be completely idle.
    /// This should not be used as a synchronization measure, except on exit.
    /// # Errors
    /// This function call can be a source of `VK_ERROR_DEVICE_LOST`. This is typically due to a couple main reasons:
    /// * Previously submitted GPU work took too long, causing a driver reset. Try splitting very large workloads into multiple submits.
    /// * Previously submitted GPU work crashed the GPU due to an out of bounds memory read or related error. Try launching the program
    ///   with GPU-assisted validation through the Vulkan configurator.
    /// * Previously submitted GPU work crashed due to invalid API usage. Make sure the validation layers are on and no invalid pointers
    ///   are being passed to Vulkan calls.
    /// # Example
    /// ```
    /// # use phobos::*;
    /// # use anyhow::Result;
    ///
    /// fn submit_some_gpu_work_and_wait(device: Device) -> Result<()> {
    ///     // ... gpu work
    ///     device.wait_idle()?;
    ///     // Work is now complete
    ///     Ok(())
    /// }
    /// ```
    pub fn wait_idle(&self) -> Result<()> {
        unsafe { Ok(self.inner.handle.device_wait_idle()?) }
    }

    /// Get unsafe access to the underlying `VkDevice` handle
    /// # Safety
    /// * The caller should not call `vkDestroyDevice` on this.
    /// * This handle is valid as long as there is a copy of `self` alive.
    /// * This can be used to do raw Vulkan calls. Modifying phobos objects through this
    ///   can put the system in an undefined state.
    pub unsafe fn handle(&self) -> ash::Device {
        self.inner.handle.clone()
    }

    /// Get the queue families we requested on this device. This is needed when using
    /// `VK_SHARING_MODE_CONCURRENT` on buffers and images.
    pub fn queue_families(&self) -> &[u32] {
        self.inner.queue_families.as_slice()
    }

    /// Get the physical device properties. This can be queried to check things such as the driver and GPU name,
    /// as well as API limitations.
    /// # Example
    /// ```
    /// # use phobos::*;
    /// use std::ffi::{CStr};
    /// fn list_device_info(device: Device) {
    ///     let properties = device.properties();
    ///     // SAFETY: The Vulkan API is guaranteed to return a null-terminated string.
    ///     let name = unsafe {
    ///         CStr::from_ptr(properties.device_name.as_ptr())
    ///     };
    ///     println!("Device name: {name:?}");
    ///     println!("Max bound descriptor sets: {}", properties.limits.max_bound_descriptor_sets);
    ///     // etc.
    /// }
    /// ```
    pub fn properties(&self) -> &vk::PhysicalDeviceProperties {
        &self.inner.properties
    }

    /// Check if a device extension is enabled.
    /// # Example
    /// ```
    /// # use phobos::*;
    /// # use phobos::core::device::ExtensionID;
    /// fn has_extended_dynamic_state(device: Device) -> bool {
    ///     device.is_extension_enabled(ExtensionID::ExtendedDynamicState3)
    /// }
    /// ```
    pub fn is_extension_enabled(&self, ext: ExtensionID) -> bool {
        self.inner.extensions.contains(&ext)
    }

    /// Access to the function pointers for `VK_EXT_dynamic_state_3`
    /// Returns `None` if the extension was not enabled or not available.
    /// # Example
    /// ```
    /// # use phobos::*;
    /// # use anyhow::Result;
    /// # use ash::extensions::ext::ExtendedDynamicState3;
    /// fn set_wireframe(device: Device, cmd: vk::CommandBuffer) -> Result<()> {
    ///     match device.dynamic_state3() {
    ///         None => {
    ///             println!("VK_EXT_extended_dynamic_state3 not enabled!");
    ///             Ok(())
    ///         },
    ///         Some(ext) => {
    ///             // SAFETY: Vulkan API call.
    ///             unsafe { ext.cmd_set_polygon_mode(cmd, vk::PolygonMode::LINE) };
    ///             Ok(())
    ///         }
    ///     }
    /// }
    /// ```
    pub fn dynamic_state3(&self) -> Option<&ash::extensions::ext::ExtendedDynamicState3> {
        self.inner.dynamic_state3.as_ref()
    }

    /// True we only have a single queue, and thus the sharing mode for resources is always EXCLUSIVE.
    /// Not extremely useful on the user side, but maybe you want to know whether one physical queue is being multiplexed
    /// behind your back.
    pub fn is_single_queue(&self) -> bool {
        self.inner.queue_families.len() == 1
    }
}

impl Deref for Device {
    type Target = ash::Device;

    fn deref(&self) -> &Self::Target {
        &self.inner.handle
    }
}

impl Drop for DeviceInner {
    fn drop(&mut self) {
        #[cfg(feature = "log-objects")]
        trace!("Destroying VkDevice {:p}", self.handle.handle());
        unsafe {
            self.handle.destroy_device(None);
        }
    }
}
