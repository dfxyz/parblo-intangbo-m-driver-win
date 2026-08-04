use anyhow::{Context, Result, anyhow};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_VALUE_CAPS, HidD_FreePreparsedData, HidD_GetAttributes,
    HidD_GetHidGuid, HidD_GetPreparsedData, HidP_GetCaps, HidP_GetValueCaps, HidP_Input,
    PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GetLastError, HANDLE, NTSTATUS, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, OpenEventW, ResetEvent, SetEvent, WaitForSingleObject,
};
use windows::core::{HSTRING, PCWSTR};

const HIDP_STATUS_SUCCESS: NTSTATUS = NTSTATUS(0x0011_0000);

pub struct HidInterface {
    pub path: Vec<u16>,
    pub usage_page: u16,
    pub input_len: usize,
    pub output_len: usize,
}

/// 笔的量程；取自标准数位板接口的报告描述符
#[derive(Clone, Copy)]
pub struct PenRanges {
    pub x_max: u16,
    pub y_max: u16,
    pub pressure_max: u16,
}
impl Default for PenRanges {
    /// 设备不在场时的兜底值，取自实机采集
    fn default() -> Self {
        Self {
            x_max: 28800,
            y_max: 16200,
            pressure_max: 8191,
        }
    }
}

struct DeviceInfoList(HDEVINFO);
impl Drop for DeviceInfoList {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

struct PreparsedData(PHIDP_PREPARSED_DATA);
impl Drop for PreparsedData {
    fn drop(&mut self) {
        unsafe {
            let _ = HidD_FreePreparsedData(self.0);
        }
    }
}

fn open_raw(path: &[u16], access: u32, overlapped: bool) -> Result<HANDLE> {
    let flags = if overlapped {
        FILE_FLAG_OVERLAPPED
    } else {
        Default::default()
    };
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
    .context("CreateFileW")?;
    Ok(handle)
}

fn query_caps(handle: HANDLE) -> Option<(u16, u16, HIDP_CAPS)> {
    let mut attributes = HIDD_ATTRIBUTES {
        Size: size_of::<HIDD_ATTRIBUTES>() as u32,
        ..Default::default()
    };
    unsafe {
        if !HidD_GetAttributes(handle, &mut attributes) {
            return None;
        }
    }

    let mut preparsed = PHIDP_PREPARSED_DATA::default();
    unsafe {
        if !HidD_GetPreparsedData(handle, &mut preparsed) {
            return None;
        }
    }
    let preparsed = PreparsedData(preparsed);

    let mut caps = HIDP_CAPS::default();
    let status = unsafe { HidP_GetCaps(preparsed.0, &mut caps) };
    if status != HIDP_STATUS_SUCCESS {
        return None;
    }
    Some((attributes.VendorID, attributes.ProductID, caps))
}

/// 枚举指定设备的所有HID顶层集合
pub fn enumerate(vendor_id: u16, product_id: u16) -> Result<Vec<HidInterface>> {
    let guid = unsafe { HidD_GetHidGuid() };
    let device_info_list = unsafe {
        SetupDiGetClassDevsW(
            Some(&guid),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }
    .context("SetupDiGetClassDevsW")?;
    let device_info_list = DeviceInfoList(device_info_list);

    let mut result = Vec::new();
    for index in 0.. {
        let mut interface_data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        let found = unsafe {
            SetupDiEnumDeviceInterfaces(
                device_info_list.0,
                None,
                &guid,
                index,
                &mut interface_data,
            )
        };
        if found.is_err() {
            break;
        }

        let mut required = 0u32;
        unsafe {
            let _ = SetupDiGetDeviceInterfaceDetailW(
                device_info_list.0,
                &interface_data,
                None,
                0,
                Some(&mut required),
                None,
            );
        }
        if required == 0 {
            continue;
        }

        let mut buffer = vec![0u8; required as usize];
        let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
        unsafe {
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(
                device_info_list.0,
                &interface_data,
                Some(detail),
                required,
                None,
                None,
            )
            .is_err()
            {
                continue;
            }
        }

        let path = unsafe {
            let ptr = (*detail).DevicePath.as_ptr();
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let mut path = std::slice::from_raw_parts(ptr, len).to_vec();
            path.push(0);
            path
        };

        let handle = match open_raw(&path, 0, false) {
            Ok(handle) => handle,
            Err(_) => continue,
        };
        let caps = query_caps(handle);
        unsafe {
            let _ = CloseHandle(handle);
        }

        let Some((vid, pid, caps)) = caps else {
            continue;
        };
        if vid != vendor_id || pid != product_id {
            continue;
        }
        result.push(HidInterface {
            path,
            usage_page: caps.UsagePage,
            input_len: caps.InputReportByteLength as usize,
            output_len: caps.OutputReportByteLength as usize,
        });
    }
    Ok(result)
}

/// 从数位板接口（用途页0x0d、用途0x02）的报告描述符中读取笔的量程
pub fn read_pen_ranges(vendor_id: u16, product_id: u16) -> Result<PenRanges> {
    let interfaces = enumerate(vendor_id, product_id).context("枚举HID接口失败")?;
    for interface in &interfaces {
        if interface.usage_page != 0x0d {
            continue;
        }
        let handle = open_raw(&interface.path, 0, false)?;
        let ranges = read_ranges_from_handle(handle);
        unsafe {
            let _ = CloseHandle(handle);
        }
        if let Some(ranges) = ranges {
            return Ok(ranges);
        }
    }
    Err(anyhow!("找不到数位板接口，无法读取笔的量程"))
}

fn read_ranges_from_handle(handle: HANDLE) -> Option<PenRanges> {
    let mut preparsed = PHIDP_PREPARSED_DATA::default();
    unsafe {
        if !HidD_GetPreparsedData(handle, &mut preparsed) {
            return None;
        }
    }
    let preparsed = PreparsedData(preparsed);

    let mut caps = HIDP_CAPS::default();
    if unsafe { HidP_GetCaps(preparsed.0, &mut caps) } != HIDP_STATUS_SUCCESS {
        return None;
    }
    if caps.UsagePage != 0x0d || caps.Usage != 0x02 {
        return None;
    }

    let mut count = caps.NumberInputValueCaps;
    let mut value_caps = vec![HIDP_VALUE_CAPS::default(); count as usize];
    let status = unsafe {
        HidP_GetValueCaps(HidP_Input, value_caps.as_mut_ptr(), &mut count, preparsed.0)
    };
    if status != HIDP_STATUS_SUCCESS {
        return None;
    }

    let mut x_max = 0;
    let mut y_max = 0;
    let mut pressure_max = 0;
    for value in value_caps.iter().take(count as usize) {
        let usage = unsafe { value.Anonymous.NotRange.Usage };
        match (value.UsagePage, usage) {
            (0x01, 0x30) => x_max = value.LogicalMax,
            (0x01, 0x31) => y_max = value.LogicalMax,
            (0x0d, 0x30) => pressure_max = value.LogicalMax,
            _ => {}
        }
    }
    if x_max <= 0 || y_max <= 0 || pressure_max <= 0 {
        return None;
    }
    Some(PenRanges {
        x_max: x_max as u16,
        y_max: y_max as u16,
        pressure_max: pressure_max as u16,
    })
}

pub struct HidDevice {
    handle: HANDLE,
    output_len: usize,
}
impl HidDevice {
    pub fn open(interface: &HidInterface) -> Result<Self> {
        let handle = open_raw(
            &interface.path,
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            true,
        )
        .context("无法打开HID设备")?;
        Ok(Self {
            handle,
            output_len: interface.output_len,
        })
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }

    /// 写出一份输出报告；不足的部分补零到该集合要求的长度
    pub fn write_report(&self, data: &[u8]) -> Result<()> {
        if data.len() > self.output_len {
            return Err(anyhow!(
                "报告长度{}超过该集合允许的{}",
                data.len(),
                self.output_len
            ));
        }
        let mut buf = vec![0u8; self.output_len];
        buf[..data.len()].copy_from_slice(data);

        let event = Event::new()?;
        let mut overlapped = OVERLAPPED {
            hEvent: event.handle(),
            ..Default::default()
        };
        let pending = unsafe { WriteFile(self.handle, Some(&buf), None, Some(&mut overlapped)) };
        if let Err(e) = pending {
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(e).context("WriteFile");
            }
        }
        let mut written = 0u32;
        unsafe { GetOverlappedResult(self.handle, &overlapped, &mut written, true) }
            .context("GetOverlappedResult(write)")?;
        Ok(())
    }

    /// 阻塞读取一份输入报告；仅用于握手阶段
    pub fn read_report(&self, len: usize, timeout_ms: u32) -> Result<Vec<u8>> {
        let event = Event::new()?;
        let mut overlapped = OVERLAPPED {
            hEvent: event.handle(),
            ..Default::default()
        };
        let mut buf = vec![0u8; len];
        let pending = unsafe { ReadFile(self.handle, Some(&mut buf), None, Some(&mut overlapped)) };
        if let Err(e) = pending {
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(e).context("ReadFile");
            }
        }

        let waited = unsafe { WaitForSingleObject(event.handle(), timeout_ms) };
        if waited != WAIT_OBJECT_0 {
            unsafe {
                let _ = CancelIoEx(self.handle, Some(&overlapped));
            }
            return Err(anyhow!("读取输入报告超时"));
        }

        let mut read = 0u32;
        unsafe { GetOverlappedResult(self.handle, &overlapped, &mut read, false) }
            .context("GetOverlappedResult(read)")?;
        buf.truncate(read as usize);
        Ok(buf)
    }
}
impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = CancelIoEx(self.handle, None);
            let _ = CloseHandle(self.handle);
        }
    }
}

pub struct Event(HANDLE);
/// 事件句柄可以安全地跨线程使用
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Event {
    pub fn new() -> Result<Self> {
        let handle =
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.context("CreateEventW")?;
        Ok(Self(handle))
    }

    /// 具名的自动重置事件，供跨进程通知使用
    pub fn named(name: &str) -> Result<Self> {
        let name = HSTRING::from(name);
        let handle = unsafe { CreateEventW(None, false, false, PCWSTR(name.as_ptr())) }
            .context("CreateEventW(named)")?;
        Ok(Self(handle))
    }

    /// 打开已有的具名事件；仅用于向另一个实例发信号
    pub fn open(name: &str) -> Result<Self> {
        let name = HSTRING::from(name);
        let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) }
            .context("OpenEventW")?;
        Ok(Self(handle))
    }

    pub fn handle(&self) -> HANDLE {
        self.0
    }

    pub fn reset(&self) {
        unsafe {
            let _ = ResetEvent(self.0);
        }
    }

    pub fn set(&self) {
        unsafe {
            let _ = SetEvent(self.0);
        }
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 常驻的异步读取器；`event()`可以并入`WaitForMultipleObjects`的等待集合
pub struct HidReader {
    handle: HANDLE,
    event: Event,
    overlapped: Box<OVERLAPPED>,
    buf: Vec<u8>,
    pending: bool,
}
impl HidReader {
    pub fn new(device: &HidDevice, input_len: usize) -> Result<Self> {
        Ok(Self {
            handle: device.handle(),
            event: Event::new()?,
            overlapped: Box::new(OVERLAPPED::default()),
            buf: vec![0u8; input_len],
            pending: false,
        })
    }

    pub fn event(&self) -> HANDLE {
        self.event.handle()
    }

    /// 若当前没有挂起的读取则发起一次
    pub fn arm(&mut self) -> Result<()> {
        if self.pending {
            return Ok(());
        }
        self.event.reset();
        *self.overlapped = OVERLAPPED {
            hEvent: self.event.handle(),
            ..Default::default()
        };
        let result = unsafe {
            ReadFile(
                self.handle,
                Some(&mut self.buf),
                None,
                Some(self.overlapped.as_mut()),
            )
        };
        if let Err(e) = result {
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(e).context("ReadFile");
            }
        }
        self.pending = true;
        Ok(())
    }

    /// 取出已完成的读取结果
    pub fn complete(&mut self) -> Result<&[u8]> {
        let mut read = 0u32;
        let result =
            unsafe { GetOverlappedResult(self.handle, self.overlapped.as_ref(), &mut read, false) };
        self.pending = false;
        result.context("GetOverlappedResult(read)")?;
        Ok(&self.buf[..read as usize])
    }
}
impl Drop for HidReader {
    fn drop(&mut self) {
        if self.pending {
            unsafe {
                let _ = CancelIoEx(self.handle, Some(self.overlapped.as_ref()));
            }
        }
    }
}
