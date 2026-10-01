//! V4L2 multi-planar MMAP capture. The v4l 0.14 arena handles only m.offset,
//! whereas this API requires a separate mapping for each m.planes element.

use std::{
    io, mem, ptr,
    ptr::NonNull,
    slice,
    sync::Arc,
    time::{Duration, Instant},
};

use iriscope_core::{
    camera::{
        MAX_CAMERA_RAW_BYTES, StreamConfiguration, camera_frame_byte_limit,
        validate_camera_frame_bytes,
    },
    capabilities::{FrameRate, PixelFormat, Resolution},
};
use v4l::{
    Device,
    buffer::{Metadata, Type as BufferType},
    device::Handle,
    format::FourCC,
    v4l_sys::{
        v4l2_buffer, v4l2_fmtdesc, v4l2_format, v4l2_plane, v4l2_requestbuffers, v4l2_streamparm,
    },
    v4l2,
};

const CAPTURE_TYPE: u32 = BufferType::VideoCaptureMplane as u32;
const MEMORY_MMAP: u32 = 1;
const MAX_PLANES: usize = 8;
const REQUESTED_BUFFERS: u32 = 4;
const MAX_BUFFERS: u32 = 8;
const MAX_FRAME_BYTES: usize = MAX_CAMERA_RAW_BYTES;
const MAX_MAPPING_BYTES: usize = 256 * 1024 * 1024;
const MAX_DRAINED_FRAMES: usize = 64;
const POLL_IN: i16 = 1;
const BUFFER_ERROR: u32 = 0x40;

#[derive(Clone, Copy, Debug)]
struct PlaneLayout {
    stride: usize,
    size: usize,
}

#[derive(Clone, Debug)]
struct FrameLayout {
    resolution: Resolution,
    pixel_format: PixelFormat,
    planes: Vec<PlaneLayout>,
}

/// Enumerates the buffer API actually used by the multi-planar node.
pub(super) fn enum_formats(device: &Device) -> io::Result<Vec<FourCC>> {
    let mut formats = Vec::new();
    for index in 0..256 {
        // SAFETY: Zero is the documented initial value of reserved ioctl fields.
        let mut format: v4l2_fmtdesc = unsafe { mem::zeroed() };
        format.type_ = CAPTURE_TYPE;
        format.index = index;
        // SAFETY: The initialized structure remains valid for this synchronous ioctl.
        match unsafe {
            v4l2::ioctl(
                device.handle().fd(),
                v4l2::vidioc::VIDIOC_ENUM_FMT,
                (&raw mut format).cast(),
            )
        } {
            Ok(()) => formats.push(FourCC::from(format.pixelformat)),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => return Ok(formats),
            Err(error) => return Err(error),
        }
    }
    Err(invalid(
        "multi-planar driver advertised too many pixel formats",
    ))
}

fn candidates(format: &PixelFormat) -> io::Result<Vec<FourCC>> {
    match format {
        PixelFormat::Mjpeg => Ok(vec![FourCC::new(b"MJPG")]),
        PixelFormat::Yuyv => Ok(vec![FourCC::new(b"YUYV"), FourCC::new(b"YUY2")]),
        PixelFormat::Nv12 => Ok(vec![FourCC::new(b"NV12"), FourCC::new(b"NM12")]),
        PixelFormat::Bgra8 => Ok(vec![FourCC::new(b"BGRA"), FourCC::new(b"BGR4")]),
        _ => Err(invalid("unsupported multi-planar pixel layout")),
    }
}

fn negotiate_layout(
    device: &Device,
    configuration: &StreamConfiguration,
) -> io::Result<FrameLayout> {
    let mut last_error = invalid("no compatible multi-planar format");
    for fourcc in candidates(&configuration.pixel_format)? {
        // SAFETY: All reserved fields must be zero for VIDIOC_S_FMT.
        let mut format: v4l2_format = unsafe { mem::zeroed() };
        format.type_ = CAPTURE_TYPE;
        // pix_mp is the union arm for VIDEO_CAPTURE_MPLANE; these writes
        // initialize fields without reading an inactive union arm.
        format.fmt.pix_mp.width = configuration.resolution.width;
        format.fmt.pix_mp.height = configuration.resolution.height;
        format.fmt.pix_mp.pixelformat = u32::from(fourcc);
        // SAFETY: Correct type and live device handle, writable format pointer.
        if let Err(error) = unsafe {
            v4l2::ioctl(
                device.handle().fd(),
                v4l2::vidioc::VIDIOC_S_FMT,
                (&raw mut format).cast(),
            )
        } {
            last_error = error;
            continue;
        }
        // SAFETY: S_FMT filled the pix_mp arm for the selected buffer type.
        let applied = unsafe { format.fmt.pix_mp };
        let applied_fourcc = FourCC::from(applied.pixelformat);
        let width = applied.width;
        let height = applied.height;
        if applied.field != 1 {
            last_error = invalid("interlaced multi-planar images are not supported");
            continue;
        }
        let pixel_format = super::platform::map_pixel_format(applied_fourcc);
        if Resolution::new(width, height) != configuration.resolution
            || pixel_format != configuration.pixel_format
        {
            last_error =
                invalid("multi-planar driver changed the requested dimensions or pixel format");
            continue;
        }
        let planes = applied.plane_fmt;
        let count = usize::from(applied.num_planes);
        if count == 0 || count > MAX_PLANES {
            last_error = invalid("invalid multi-planar plane count");
            continue;
        }
        let layouts = planes[..count]
            .iter()
            .map(|plane| PlaneLayout {
                stride: plane.bytesperline as usize,
                size: plane.sizeimage as usize,
            })
            .collect();
        let layout = FrameLayout {
            resolution: configuration.resolution,
            pixel_format,
            planes: layouts,
        };
        match layout.validate(applied_fourcc) {
            Ok(()) => return Ok(layout),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn negotiate_rate(device: &Device, requested: FrameRate) -> io::Result<FrameRate> {
    // SAFETY: ioctl reserved fields and union are initialized to zero.
    let mut parameters: v4l2_streamparm = unsafe { mem::zeroed() };
    parameters.type_ = CAPTURE_TYPE;
    // capture is the correct arm; these assignments initialize its fields.
    parameters.parm.capture.timeperframe.numerator = requested.denominator();
    parameters.parm.capture.timeperframe.denominator = requested.numerator();
    // SAFETY: Parameters and live device handle are valid for the ioctl.
    let set_result = unsafe {
        v4l2::ioctl(
            device.handle().fd(),
            v4l2::vidioc::VIDIOC_S_PARM,
            (&raw mut parameters).cast(),
        )
    };
    if set_result.is_err() {
        // SAFETY: Same buffer type, reading into a live initialized structure.
        unsafe {
            v4l2::ioctl(
                device.handle().fd(),
                v4l2::vidioc::VIDIOC_G_PARM,
                (&raw mut parameters).cast(),
            )
        }?;
    }
    // SAFETY: A successful S/G_PARM populated capture.timeperframe.
    let interval = unsafe { parameters.parm.capture.timeperframe };
    FrameRate::new(interval.denominator, interval.numerator)
        .ok_or_else(|| invalid("multi-planar driver returned invalid frame interval"))
}

struct Mapping {
    pointer: NonNull<u8>,
    length: usize,
}

// SAFETY: A mapping belongs exclusively to its stream. It is read only after
// DQBUF and no borrowed slice escapes next_frame or survives QBUF/STREAMOFF.
unsafe impl Send for Mapping {}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: This mapping was returned by mmap and is released exactly once.
        let _ = unsafe { v4l2::munmap(self.pointer.as_ptr().cast(), self.length) };
    }
}

pub(super) struct MultiPlaneStream {
    handle: Arc<Handle>,
    layout: FrameLayout,
    buffers: Vec<Vec<Mapping>>,
    allocated: bool,
}

impl MultiPlaneStream {
    pub(super) fn start(
        device: &Device,
        requested: &StreamConfiguration,
    ) -> io::Result<(Self, StreamConfiguration)> {
        camera_frame_byte_limit(&requested.pixel_format, requested.resolution)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let layout = negotiate_layout(device, requested)?;
        let rate = negotiate_rate(device, requested.frame_rate)?;
        let mut stream = Self {
            handle: device.handle(),
            layout,
            buffers: Vec::new(),
            allocated: false,
        };
        stream.allocate()?;
        for index in 0..stream.buffers.len() {
            stream.queue(index)?;
        }
        let mut buffer_type = CAPTURE_TYPE;
        // SAFETY: The type matches all configured/queued buffers for this handle.
        unsafe {
            v4l2::ioctl(
                stream.handle.fd(),
                v4l2::vidioc::VIDIOC_STREAMON,
                (&raw mut buffer_type).cast(),
            )
        }?;
        Ok((
            stream,
            StreamConfiguration {
                frame_rate: rate,
                ..requested.clone()
            },
        ))
    }

    fn allocate(&mut self) -> io::Result<()> {
        let frame_limit =
            camera_frame_byte_limit(&self.layout.pixel_format, self.layout.resolution)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        // SAFETY: Zeroing initializes the reserved fields in requestbuffers.
        let mut request: v4l2_requestbuffers = unsafe { mem::zeroed() };
        request.type_ = CAPTURE_TYPE;
        request.memory = MEMORY_MMAP;
        request.count = REQUESTED_BUFFERS;
        // SAFETY: Device handle and request pointer are valid for synchronous ioctl.
        unsafe {
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_REQBUFS,
                (&raw mut request).cast(),
            )
        }?;
        self.allocated = true;
        if request.count == 0 || request.count > MAX_BUFFERS {
            return Err(invalid("multi-planar buffer count exceeds safe limits"));
        }
        let mut total = 0_usize;
        for index in 0..request.count {
            // SAFETY: Reserved plane fields must be zero.
            let mut planes = unsafe { mem::zeroed::<[v4l2_plane; MAX_PLANES]>() };
            let mut buffer = buffer_descriptor(index, &mut planes, self.layout.planes.len());
            // SAFETY: m.planes points at a live MAX_PLANES element array, capacity advertised in length.
            unsafe {
                v4l2::ioctl(
                    self.handle.fd(),
                    v4l2::vidioc::VIDIOC_QUERYBUF,
                    (&raw mut buffer).cast(),
                )
            }?;
            if buffer.length as usize != self.layout.planes.len() {
                return Err(invalid(
                    "multi-planar buffer plane count differs from negotiated format",
                ));
            }
            self.buffers.push(Vec::new());
            for (plane_index, plane) in planes[..self.layout.planes.len()].iter().enumerate() {
                let length = plane.length as usize;
                total = total
                    .checked_add(length)
                    .ok_or_else(|| invalid("multi-planar mapping size overflow"))?;
                if length < self.layout.planes[plane_index].size
                    || length > frame_limit
                    || total > MAX_MAPPING_BYTES
                {
                    return Err(invalid("multi-planar mappings exceed safe size limits"));
                }
                // SAFETY: QUERYBUF provided the MMAP offset for this plane.
                let offset = unsafe { plane.m.mem_offset };
                #[allow(clippy::useless_conversion)]
                let offset = i64::from(offset)
                    .try_into()
                    .map_err(|_| invalid("multi-planar mapping offset out of range"))?;
                // SAFETY: Length was bounded, offset returned by the driver. Linux constants
                // PROT_READ|PROT_WRITE=3 and MAP_SHARED=1 are ABI values.
                let pointer =
                    unsafe { v4l2::mmap(ptr::null_mut(), length, 3, 1, self.handle.fd(), offset) }?;
                let Some(pointer) = NonNull::new(pointer.cast()) else {
                    // SAFETY: The successful mmap result must still be released
                    // if its address cannot be represented by our non-null owner.
                    let _ = unsafe { v4l2::munmap(pointer, length) };
                    return Err(invalid("driver returned a null mapping"));
                };
                self.buffers
                    .last_mut()
                    .expect("buffer was inserted")
                    .push(Mapping { pointer, length });
            }
        }
        Ok(())
    }

    fn queue(&self, index: usize) -> io::Result<()> {
        // SAFETY: All reserved plane fields are zeroed.
        let mut planes = unsafe { mem::zeroed::<[v4l2_plane; MAX_PLANES]>() };
        for (plane, mapping) in planes.iter_mut().zip(&self.buffers[index]) {
            plane.length =
                u32::try_from(mapping.length).map_err(|_| invalid("plane mapping too large"))?;
        }
        let mut buffer = buffer_descriptor(
            u32::try_from(index).map_err(|_| invalid("invalid buffer index"))?,
            &mut planes,
            self.layout.planes.len(),
        );
        // SAFETY: This is an owned mapped capture buffer with a valid live plane array.
        unsafe {
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_QBUF,
                (&raw mut buffer).cast(),
            )
        }
    }

    pub(super) fn next_frame(&mut self, timeout: Duration) -> io::Result<(Arc<[u8]>, Metadata)> {
        let started = Instant::now();
        for attempt in 0..MAX_DRAINED_FRAMES {
            let remaining = timeout.saturating_sub(started.elapsed());
            let millis = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
            if self.handle.poll(POLL_IN, millis)? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "multi-planar frame timeout",
                ));
            }
            // SAFETY: Zero reserved fields; DQBUF will populate actual plane sizes.
            let mut planes = unsafe { mem::zeroed::<[v4l2_plane; MAX_PLANES]>() };
            let mut buffer = buffer_descriptor(0, &mut planes, self.layout.planes.len());
            // SAFETY: m.planes points at the live initialized array for this ioctl.
            let dequeued = unsafe {
                v4l2::ioctl(
                    self.handle.fd(),
                    v4l2::vidioc::VIDIOC_DQBUF,
                    (&raw mut buffer).cast(),
                )
            };
            if let Err(error) = dequeued {
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::Interrupted
                {
                    continue;
                }
                return Err(error);
            }
            let index = buffer.index as usize;
            if index >= self.buffers.len() || buffer.length as usize != self.layout.planes.len() {
                return Err(invalid(
                    "multi-planar driver returned invalid buffer index or plane count",
                ));
            }
            let stale = attempt + 1 < MAX_DRAINED_FRAMES && self.handle.poll(POLL_IN, 0)? != 0;
            if stale || buffer.flags & BUFFER_ERROR != 0 {
                self.queue(index)?;
                continue;
            }
            let result = self.copy_frame(index, &planes);
            // Requeue even if validating/copying the driver's data failed.
            self.queue(index)?;
            let payload = result?;
            let metadata = Metadata {
                bytesused: u32::try_from(payload.len())
                    .map_err(|_| invalid("frame payload too large"))?,
                flags: buffer.flags.into(),
                field: buffer.field,
                timestamp: buffer.timestamp.into(),
                sequence: buffer.sequence,
            };
            return Ok((Arc::from(payload), metadata));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "multi-planar frame queue did not produce a valid fresh frame",
        ))
    }

    fn copy_frame(&self, index: usize, planes: &[v4l2_plane; MAX_PLANES]) -> io::Result<Vec<u8>> {
        let mut payloads = Vec::new();
        for (mapping, plane) in self.buffers[index].iter().zip(planes) {
            let range = payload_range(
                mapping.length,
                plane.bytesused as usize,
                plane.data_offset as usize,
            )?;
            // SAFETY: The buffer is dequeued and owned by userspace. Bounds are
            // checked against the live mapping; no slice survives copying/requeueing.
            let payload = unsafe {
                slice::from_raw_parts(mapping.pointer.as_ptr().add(range.start), range.len())
            };
            payloads.push(payload);
        }
        self.layout.pack(&payloads)
    }
}

impl Drop for MultiPlaneStream {
    fn drop(&mut self) {
        if !self.allocated {
            return;
        }
        let mut buffer_type = CAPTURE_TYPE;
        // SAFETY: Correct buffer type for the allocated stream. Errors during
        // unplug/shutdown are ignored; mappings are still released without panic.
        let _ = unsafe {
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_STREAMOFF,
                (&raw mut buffer_type).cast(),
            )
        };
        self.buffers.clear();
        // SAFETY: Requesting zero buffers releases driver allocations.
        let mut request: v4l2_requestbuffers = unsafe { mem::zeroed() };
        request.type_ = CAPTURE_TYPE;
        request.memory = MEMORY_MMAP;
        let _ = unsafe {
            v4l2::ioctl(
                self.handle.fd(),
                v4l2::vidioc::VIDIOC_REQBUFS,
                (&raw mut request).cast(),
            )
        };
    }
}

fn buffer_descriptor(
    index: u32,
    planes: &mut [v4l2_plane; MAX_PLANES],
    count: usize,
) -> v4l2_buffer {
    // SAFETY: Zeroing initializes every reserved field in the C structure.
    let mut buffer: v4l2_buffer = unsafe { mem::zeroed() };
    buffer.type_ = CAPTURE_TYPE;
    buffer.memory = MEMORY_MMAP;
    buffer.index = index;
    buffer.length = u32::try_from(count).unwrap_or(0);
    buffer.m.planes = planes.as_mut_ptr();
    buffer
}

fn payload_range(length: usize, used: usize, offset: usize) -> io::Result<std::ops::Range<usize>> {
    if offset > used || used > length {
        return Err(invalid(
            "multi-planar data offset or bytesused exceeds its mapping",
        ));
    }
    Ok(offset..used)
}

impl FrameLayout {
    fn validate(&self, fourcc: FourCC) -> io::Result<()> {
        let frame_limit = camera_frame_byte_limit(&self.pixel_format, self.resolution)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let expected = if fourcc.repr == *b"NM12" { 2 } else { 1 };
        if self.planes.len() != expected {
            return Err(invalid(
                "unsupported number of planes for this pixel format",
            ));
        }
        let width = self.resolution.width as usize;
        let height = self.resolution.height as usize;
        if width == 0 || height == 0 {
            return Err(invalid("zero multi-planar frame dimension"));
        }
        if matches!(self.pixel_format, PixelFormat::Nv12 | PixelFormat::Yuyv) && width % 2 != 0 {
            return Err(invalid("YUV frame width must be even"));
        }
        if self.pixel_format == PixelFormat::Nv12 && height % 2 != 0 {
            return Err(invalid("NV12 frame height must be even"));
        }
        let total = self.planes.iter().try_fold(0_usize, |total, plane| {
            if plane.size == 0 || plane.size > frame_limit {
                return Err(invalid("multi-planar plane size exceeds limits"));
            }
            total
                .checked_add(plane.size)
                .ok_or_else(|| invalid("multi-planar frame size overflow"))
        })?;
        if total > frame_limit {
            return Err(invalid("multi-planar frame exceeds its size limit"));
        }
        if self.pixel_format == PixelFormat::Mjpeg {
            return Ok(());
        }
        let row = match self.pixel_format {
            PixelFormat::Yuyv => width.checked_mul(2),
            PixelFormat::Bgra8 => width.checked_mul(4),
            PixelFormat::Nv12 => Some(width),
            _ => None,
        }
        .ok_or_else(|| invalid("unsupported multi-planar pixel format"))?;
        for (index, plane) in self.planes.iter().enumerate() {
            let rows = if self.pixel_format == PixelFormat::Nv12 {
                if self.planes.len() == 1 {
                    height
                        .checked_add(height / 2)
                        .ok_or_else(|| invalid("NV12 height overflow"))?
                } else if index == 0 {
                    height
                } else {
                    height / 2
                }
            } else {
                height
            };
            let needed = plane
                .stride
                .checked_mul(rows)
                .ok_or_else(|| invalid("multi-planar stride size overflow"))?;
            if plane.stride < row || needed > plane.size {
                return Err(invalid(
                    "multi-planar stride or plane size is smaller than the image",
                ));
            }
        }
        Ok(())
    }

    fn pack(&self, payloads: &[&[u8]]) -> io::Result<Vec<u8>> {
        if payloads.len() != self.planes.len() {
            return Err(invalid("frame plane count mismatch"));
        }
        if self.pixel_format == PixelFormat::Mjpeg {
            validate_camera_frame_bytes(&self.pixel_format, self.resolution, payloads[0].len())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            return Ok(payloads[0].to_vec());
        }
        let width = self.resolution.width as usize;
        let height = self.resolution.height as usize;
        let mut output = Vec::new();
        match self.pixel_format {
            PixelFormat::Nv12 if self.planes.len() == 2 => {
                append_rows(
                    &mut output,
                    payloads[0],
                    self.planes[0].stride,
                    width,
                    height,
                )?;
                append_rows(
                    &mut output,
                    payloads[1],
                    self.planes[1].stride,
                    width,
                    height / 2,
                )?;
            }
            PixelFormat::Nv12 => append_rows(
                &mut output,
                payloads[0],
                self.planes[0].stride,
                width,
                height + height / 2,
            )?,
            PixelFormat::Yuyv => append_rows(
                &mut output,
                payloads[0],
                self.planes[0].stride,
                width * 2,
                height,
            )?,
            PixelFormat::Bgra8 => append_rows(
                &mut output,
                payloads[0],
                self.planes[0].stride,
                width * 4,
                height,
            )?,
            _ => return Err(invalid("unsupported multi-planar frame layout")),
        }
        Ok(output)
    }
}

fn append_rows(
    output: &mut Vec<u8>,
    source: &[u8],
    stride: usize,
    width: usize,
    rows: usize,
) -> io::Result<()> {
    let appended = width
        .checked_mul(rows)
        .ok_or_else(|| invalid("packed frame size overflow"))?;
    if output
        .len()
        .checked_add(appended)
        .is_none_or(|size| size > MAX_FRAME_BYTES)
    {
        return Err(invalid("packed multi-planar frame exceeds size limit"));
    }
    output.reserve(appended);
    for row in 0..rows {
        let start = row
            .checked_mul(stride)
            .ok_or_else(|| invalid("plane stride offset overflow"))?;
        let end = start
            .checked_add(width)
            .ok_or_else(|| invalid("plane row size overflow"))?;
        let bytes = source
            .get(start..end)
            .ok_or_else(|| invalid("multi-planar frame has a truncated image row"))?;
        output.extend_from_slice(bytes);
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12m_planes_are_flattened_without_stride_padding() {
        let layout = FrameLayout {
            resolution: Resolution::new(4, 2),
            pixel_format: PixelFormat::Nv12,
            planes: vec![
                PlaneLayout {
                    stride: 6,
                    size: 12,
                },
                PlaneLayout { stride: 4, size: 4 },
            ],
        };
        layout
            .validate(FourCC::new(b"NM12"))
            .expect("linear NV12M layout");
        let output = layout
            .pack(&[&[1, 2, 3, 4, 99, 99, 5, 6, 7, 8, 99, 99], &[9, 10, 11, 12]])
            .expect("pack");
        assert_eq!(output, (1..=12).collect::<Vec<_>>());
    }

    #[test]
    fn single_plane_nv12_in_multi_api_removes_y_and_uv_padding() {
        let layout = FrameLayout {
            resolution: Resolution::new(2, 2),
            pixel_format: PixelFormat::Nv12,
            planes: vec![PlaneLayout {
                stride: 4,
                size: 12,
            }],
        };
        layout.validate(FourCC::new(b"NV12")).expect("NV12 layout");
        assert_eq!(
            layout
                .pack(&[&[1, 2, 99, 99, 3, 4, 99, 99, 5, 6, 99, 99]])
                .expect("pack"),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn yuyv_padded_rows_and_truncated_bytes_are_checked() {
        let layout = FrameLayout {
            resolution: Resolution::new(2, 2),
            pixel_format: PixelFormat::Yuyv,
            planes: vec![PlaneLayout {
                stride: 6,
                size: 12,
            }],
        };
        layout.validate(FourCC::new(b"YUYV")).expect("YUYV layout");
        assert_eq!(
            layout
                .pack(&[&[1, 2, 3, 4, 99, 99, 5, 6, 7, 8, 99, 99]])
                .expect("pack"),
            (1..=8).collect::<Vec<_>>()
        );
        assert!(layout.pack(&[&[1, 2, 3, 4, 99, 99, 5, 6, 7]]).is_err());
    }

    #[test]
    fn data_offsets_and_plane_counts_cannot_escape_mapped_buffers() {
        assert_eq!(payload_range(12, 10, 2).expect("offset"), 2..10);
        assert!(payload_range(12, 13, 2).is_err());
        assert!(payload_range(12, 4, 5).is_err());
        let layout = FrameLayout {
            resolution: Resolution::new(2, 2),
            pixel_format: PixelFormat::Nv12,
            planes: vec![PlaneLayout { stride: 2, size: 6 }],
        };
        assert!(layout.validate(FourCC::new(b"NM12")).is_err());
        assert!(layout.pack(&[]).is_err());
    }

    #[test]
    fn odd_nv12_dimensions_and_oversized_mappings_are_rejected() {
        let odd = FrameLayout {
            resolution: Resolution::new(3, 2),
            pixel_format: PixelFormat::Nv12,
            planes: vec![PlaneLayout { stride: 3, size: 9 }],
        };
        assert!(odd.validate(FourCC::new(b"NV12")).is_err());
        let huge = FrameLayout {
            resolution: Resolution::new(2, 2),
            pixel_format: PixelFormat::Mjpeg,
            planes: vec![PlaneLayout {
                stride: 0,
                size: iriscope_core::camera::MAX_CAMERA_MJPEG_BYTES + 1,
            }],
        };
        assert!(huge.validate(FourCC::new(b"MJPG")).is_err());
        let maximum = FrameLayout {
            resolution: Resolution::new(2, 2),
            pixel_format: PixelFormat::Mjpeg,
            planes: vec![PlaneLayout {
                stride: 0,
                size: iriscope_core::camera::MAX_CAMERA_MJPEG_BYTES,
            }],
        };
        maximum
            .validate(FourCC::new(b"MJPG"))
            .expect("32 MiB MJPEG mapping is allowed");
    }
}
