use std::ffi::c_void;

use anyhow::{Result, anyhow};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_EVENT_DATA,
    CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_0,
    CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification, CM_Unregister_Notification,
    CR_SUCCESS, HCMNOTIFICATION,
};
use windows::Win32::Devices::HumanInterfaceDevice::HidD_GetHidGuid;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::SetEvent;

use crate::device::hid::Event;

/// 监听HID接口的到达事件；设备重新插上时唤醒驱动循环
pub struct DeviceNotifier {
    notification: HCMNOTIFICATION,
    event: Event,
}

impl DeviceNotifier {
    pub fn new() -> Result<Self> {
        let event = Event::new()?;
        let guid = unsafe { HidD_GetHidGuid() };
        let filter = CM_NOTIFY_FILTER {
            cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            u: CM_NOTIFY_FILTER_0 {
                DeviceInterface: CM_NOTIFY_FILTER_0_0 { ClassGuid: guid },
            },
            ..Default::default()
        };

        let mut notification = HCMNOTIFICATION::default();
        let result = unsafe {
            CM_Register_Notification(
                &filter,
                Some(event.handle().0 as *const c_void),
                Some(on_notify),
                &mut notification,
            )
        };
        if result != CR_SUCCESS {
            return Err(anyhow!("CM_Register_Notification失败: {:?}", result));
        }
        Ok(Self {
            notification,
            event,
        })
    }

    pub fn event(&self) -> HANDLE {
        self.event.handle()
    }

    pub fn reset(&self) {
        self.event.reset();
    }
}

impl Drop for DeviceNotifier {
    fn drop(&mut self) {
        unsafe {
            CM_Unregister_Notification(self.notification);
        }
    }
}

unsafe extern "system" fn on_notify(
    _notification: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    _data: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL {
        unsafe {
            let _ = SetEvent(HANDLE(context as *mut c_void));
        }
    }
    0
}
