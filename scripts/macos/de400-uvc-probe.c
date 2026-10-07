/* Opt-in direct video + physical button test for the observed DE400.
 * Requires administrator privileges because libusb captures the USB device.
 * Libraries stay in target/. No driver, daemon or firmware is installed.
 */
#include <libuvc/libuvc.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdio.h>
#include <unistd.h>

static volatile sig_atomic_t interrupted;
static atomic_uint frames, presses;
static void stop(int signal) { (void)signal; interrupted = 1; }
static void frame_received(uvc_frame_t *frame, void *context) {
    (void)context;
    unsigned count = atomic_fetch_add(&frames, 1) + 1;
    if (count <= 3 || count % 80 == 0)
        printf("USB_FRAME count=%u width=%u height=%u format=%d bytes=%zu\n",
               count, frame->width, frame->height, frame->frame_format, frame->data_bytes);
}
static void button_received(int button, int state, void *context) {
    (void)context;
    printf("USB_BUTTON interface=%d state=%d frames=%u\n", button, state, atomic_load(&frames));
    if (button == 1 && state == 1) atomic_fetch_add(&presses, 1);
}
int main(void) {
    setbuf(stdout, NULL);
    if (geteuid() != 0) { fputs("ADMIN_REQUIRED; no USB changes made.\n", stderr); return 77; }
    signal(SIGINT, stop); signal(SIGTERM, stop);
    uvc_context_t *context = NULL;
    uvc_device_t *device = NULL;
    uvc_device_handle_t *handle = NULL;
    uvc_stream_ctrl_t control = {0};
    int result = uvc_init(&context, NULL);
    printf("UVC_INIT result=%d\n", result);
    if (result) return 1;
    result = uvc_find_device(context, &device, 0x21cd, 0x603b, "VTU603EB");
    printf("UVC_FIND_EXACT_DE400 result=%d\n", result);
    if (result) goto done;
    result = uvc_open(device, &handle);
    printf("UVC_CAPTURE_OPEN result=%d\n", result);
    if (result) goto done;
    uvc_set_button_callback(handle, button_received, NULL);
    uvc_print_diag(handle, stdout);
    result = uvc_get_stream_ctrl_format_size(handle, &control, UVC_FRAME_FORMAT_YUYV, 1280, 1024, 8);
    printf("UVC_STREAM_MODE result=%d interval=%u payload=%u\n", result, control.dwFrameInterval, control.dwMaxPayloadTransferSize);
    if (result) goto done;
    result = uvc_start_streaming(handle, &control, frame_received, NULL, 0);
    printf("UVC_STREAM_START result=%d\n", result);
    if (result) goto done;
    puts("UVC_READY: video and button observed together for at most 60 seconds.");
    for (unsigned tick = 0; tick < 600 && !interrupted; tick++) usleep(100000);
    uvc_stop_streaming(handle);
done:
    if (handle) uvc_close(handle);
    if (device) uvc_unref_device(device);
    uvc_exit(context);
    printf("UVC_DONE frames=%u presses=%u device_return_requested=1\n", atomic_load(&frames), atomic_load(&presses));
    return result || !atomic_load(&frames) ? 1 : 0;
}
