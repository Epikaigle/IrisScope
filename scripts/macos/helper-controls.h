/* Only advertised DE400 UVC controls are exposed. USB requests are bounded. */
#include <libusb.h>
#include <limits.h>

struct camera_control {
    uint32_t id, kind;
    uint8_t unit, selector, length;
    bool signed_value;
    int32_t minimum, maximum, step, default_value, current;
    bool read_only;
};
static struct camera_control controls[13];
static unsigned control_count;
static uint32_t read_le32(const unsigned char *bytes) {
    return (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 |
           (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
}
static int read_control(uvc_device_handle_t *handle, const struct camera_control *control,
                        uint8_t request, int32_t *value) {
    unsigned char data[4] = {0};
    int length = request == UVC_GET_INFO ? 1 : control->length;
    int result = libusb_control_transfer(uvc_get_libusb_handle(handle), 0xa1, request,
        (uint16_t)control->selector << 8, (uint16_t)control->unit << 8,
        data, (uint16_t)length, 500);
    if (result != length) return result < 0 ? result : UVC_ERROR_IO;
    uint32_t raw = read_le32(data);
    if (request != UVC_GET_INFO && control->signed_value && control->length == 2)
        *value = (int16_t)raw;
    else if (raw <= INT32_MAX) *value = (int32_t)raw;
    else return UVC_ERROR_INVALID_PARAM;
    return 0;
}
static bool valid_control_value(const struct camera_control *control, int32_t value) {
    if (value < control->minimum || value > control->maximum) return false;
    if (control->kind == 2) return value < 31 && (control->step & (1u << value));
    return control->step > 0 && (value - control->minimum) % control->step == 0;
}
static int write_control(uvc_device_handle_t *handle, const struct camera_control *control, int32_t value) {
    if (control->read_only || !valid_control_value(control, value)) return UVC_ERROR_INVALID_PARAM;
    unsigned char data[4]; little_endian(data, (uint32_t)value);
    int result = libusb_control_transfer(uvc_get_libusb_handle(handle), 0x21, UVC_SET_CUR,
        (uint16_t)control->selector << 8, (uint16_t)control->unit << 8,
        data, control->length, 500);
    return result == control->length ? 0 : (result < 0 ? result : UVC_ERROR_IO);
}
static void discover_controls(uvc_device_handle_t *handle) {
    /* id, selector, advertised bitmap bit, byte length, signed, presentation */
    static const unsigned specifications[][6] = {
        {1, 2, 0, 2, 1, 0}, {2, 3, 1, 2, 0, 0}, {3, 7, 3, 2, 0, 0},
        {4, 6, 2, 2, 1, 0}, {5, 8, 4, 2, 0, 0}, {6, 9, 5, 2, 0, 0},
        {7, 10, 6, 2, 0, 0}, {8, 11, 12, 1, 0, 1}, {9, 5, 10, 1, 0, 2},
        {10, 4, 3, 4, 0, 0}, {11, 2, 1, 1, 0, 2},
        {12, 4, 9, 2, 0, 0}, {13, 1, 8, 2, 0, 0},
    };
    const uvc_processing_unit_t *pu = uvc_get_processing_units(handle);
    const uvc_input_terminal_t *ct = uvc_get_camera_terminal(handle);
    control_count = 0;
    for (unsigned i = 0; i < 13; i++) {
        const unsigned *spec = specifications[i];
        bool terminal = spec[0] == 10 || spec[0] == 11;
        if ((terminal && (!ct || !(ct->bmControls & (1ULL << spec[2])))) ||
            (!terminal && (!pu || !(pu->bmControls & (1ULL << spec[2]))))) continue;
        struct camera_control control = {.id = spec[0], .kind = spec[5],
            .unit = terminal ? ct->bTerminalID : pu->bUnitID,
            .selector = (uint8_t)spec[1], .length = (uint8_t)spec[3],
            .signed_value = spec[4], .step = 1};
        int32_t info;
        if (read_control(handle, &control, UVC_GET_INFO, &info) || !(info & 1) ||
            read_control(handle, &control, UVC_GET_CUR, &control.current) ||
            read_control(handle, &control, UVC_GET_DEF, &control.default_value)) continue;
        control.read_only = !(info & 2);
        if (control.id == 8) { control.minimum = 0; control.maximum = 1; }
        else if (control.id == 11) {
            int32_t modes;
            if (read_control(handle, &control, UVC_GET_RES, &modes)) continue;
            control.minimum = 1; control.maximum = 8; control.step = 0;
            for (unsigned mode = 1; mode <= 8; mode <<= 1)
                if (modes & mode) control.step |= 1u << mode;
        } else {
            if (read_control(handle, &control, UVC_GET_MIN, &control.minimum) ||
                read_control(handle, &control, UVC_GET_MAX, &control.maximum) ||
                read_control(handle, &control, UVC_GET_RES, &control.step)) continue;
            if (control.id == 9) {
                if (control.minimum < 0 || control.maximum > 3) continue;
                control.step = 0;
                for (int32_t value = control.minimum; value <= control.maximum; value++) control.step |= 1u << value;
            }
        }
        if (control.minimum >= control.maximum || !valid_control_value(&control, control.current) ||
            !valid_control_value(&control, control.default_value)) continue;
        controls[control_count++] = control;
        printf("USB_CONTROL id=%u min=%d max=%d step=%d default=%d current=%d readonly=%d\n",
            control.id, control.minimum, control.maximum, control.step, control.default_value, control.current, control.read_only);
    }
}
static bool send_control_descriptors(void) {
    unsigned char data[13 * 32];
    for (unsigned i = 0; i < control_count; i++) {
        const struct camera_control *control = &controls[i];
        uint32_t values[8] = {control->id, control->kind, (uint32_t)control->minimum,
            (uint32_t)control->maximum, (uint32_t)control->step, (uint32_t)control->default_value,
            (uint32_t)control->current, control->read_only};
        for (unsigned j = 0; j < 8; j++) little_endian(data + i * 32 + j * 4, values[j]);
    }
    return packet(4, data, control_count * 32);
}
static void serve_control(int server, uid_t uid, uvc_device_handle_t *handle) {
    int fd = accept(server, NULL, NULL);
    if (fd < 0) return;
    uid_t peer; gid_t group;
    struct timeval timeout = {.tv_sec = 1};
    int no_sigpipe = 1;
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &no_sigpipe, sizeof(no_sigpipe));
    unsigned char request[12]; size_t received = 0;
    if (getpeereid(fd, &peer, &group) || peer != uid) goto done_control;
    while (received < sizeof(request)) {
        ssize_t count = recv(fd, request + received, sizeof(request) - received, 0);
        if (count <= 0) goto done_control;
        received += (size_t)count;
    }
    uint32_t operation = read_le32(request), id = read_le32(request + 4);
    int32_t value = (int32_t)read_le32(request + 8), status = UVC_ERROR_INVALID_PARAM;
    if (operation == 3 && id == 0 && value == 0) { interrupted = 1; status = 0; }
    else if (handle && operation <= 1) {
        for (unsigned i = 0; i < control_count; i++) if (controls[i].id == id) {
            status = operation == 1 ? write_control(handle, &controls[i], value) : 0;
            if (!status) status = read_control(handle, &controls[i], UVC_GET_CUR, &value);
            break;
        }
    }
    unsigned char reply[8]; little_endian(reply, (uint32_t)status); little_endian(reply + 4, (uint32_t)value);
    /* Eight bytes; MSG_NOSIGNAL equivalent above and a bounded send timeout. */
    size_t sent = 0;
    while (sent < sizeof(reply)) {
        ssize_t count = send(fd, reply + sent, sizeof(reply) - sent, 0);
        if (count <= 0) break;
        sent += (size_t)count;
    }
done_control:
    close(fd);
}
