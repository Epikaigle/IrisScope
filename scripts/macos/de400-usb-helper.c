/* USB video/button/controls helper for the DE400, authorized by the Mac launcher.
 * Each reader follows one launcher session; the installed broker starts it on demand.
 * The GUI stays under the user's account. Only this camera is captured by libusb.
 * Stream IPC: LE u32 kind + LE u32 size, then payload, with a START request.
 * Separate UID-authenticated control IPC handles get/set and shutdown.
 */
#include <libuvc/libuvc.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/select.h>
#include <sys/stat.h>
#include <pthread.h>
#include <signal.h>
#include <stdbool.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <libproc.h>
#include "helper-session.h"

static volatile sig_atomic_t interrupted;
static atomic_bool failed;
static atomic_uint frames, dropped, presses;
static pthread_mutex_t output_lock = PTHREAD_MUTEX_INITIALIZER;
static int client = -1;
static void stop(int signal) { (void)signal; interrupted = 1; }
static void little_endian(unsigned char *out, uint32_t value) {
    for (unsigned i = 0; i < 4; i++) out[i] = (unsigned char)(value >> (8 * i));
}
static bool write_all(const void *bytes, size_t length) {
    const unsigned char *data = bytes;
    while (length) {
        ssize_t count = send(client, data, length, 0);
        if (count <= 0) { atomic_store(&failed, true); return false; }
        data += count; length -= (size_t)count;
    }
    return true;
}
static bool packet(uint32_t kind, const void *data, uint32_t size) {
    unsigned char header[8]; little_endian(header, kind); little_endian(header + 4, size);
    pthread_mutex_lock(&output_lock);
    bool success = write_all(header, sizeof(header)) && write_all(data, size);
    pthread_mutex_unlock(&output_lock);
    return success;
}
#include "helper-controls.h"
static void frame_received(uvc_frame_t *frame, void *context) {
    (void)context;
    if (frame->width != 1280 || frame->height != 1024 ||
        frame->frame_format != UVC_FRAME_FORMAT_YUYV || frame->data_bytes != 2621440) {
        unsigned count = atomic_fetch_add(&dropped, 1) + 1;
        if (count <= 3) printf("USB_INCOMPLETE_FRAME bytes=%zu dropped=%u\n", frame->data_bytes, count);
        return;
    }
    if (!packet(1, frame->data, (uint32_t)frame->data_bytes)) return;
    unsigned count = atomic_fetch_add(&frames, 1) + 1;
    if (count <= 3 || count % 80 == 0) printf("USB_FRAME count=%u 1280x1024 YUYV\n", count);
}
static void button_received(int button, int state, void *context) {
    (void)context;
    if (button != 1 || (state != 0 && state != 1)) return;
    unsigned char report[4] = {2, 1, 0, (unsigned char)state};
    packet(2, report, sizeof(report));
    printf("USB_BUTTON state=%d frames=%u\n", state, atomic_load(&frames));
    if (state) atomic_fetch_add(&presses, 1);
}
static int create_server(const char *path, uid_t uid) {
    struct sockaddr_un address = {.sun_family = AF_UNIX};
    if (strlen(path) >= sizeof(address.sun_path)) return -1;
    strcpy(address.sun_path, path);
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return -1;
    /* Filesystem operations use the GUI user's permissions, including cleanup. */
    if (seteuid(uid)) { close(fd); return -1; }
    int result = bind(fd, (struct sockaddr *)&address, sizeof(address));
    int saved = errno;
    if (seteuid(0)) { close(fd); return -1; }
    if (result || listen(fd, 4)) { errno = saved; close(fd); return -1; }
    return fd;
}
static bool parent_alive(pid_t pid, const struct proc_bsdinfo *original) {
    if (!pid) return true;
    struct proc_bsdinfo current;
    return proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &current, sizeof(current)) == sizeof(current) &&
        current.pbi_uid == original->pbi_uid && current.pbi_start_tvsec == original->pbi_start_tvsec &&
        current.pbi_start_tvusec == original->pbi_start_tvusec;
}
static unsigned long parse_number(const char *text) {
    char *end; errno = 0;
    unsigned long value = strtoul(text, &end, 10);
    return !*text || *end || errno ? 0 : value;
}
int main(int argc, char **argv) {
    setbuf(stdout, NULL);
    bool persistent = argc == 5 && !strcmp(argv[3], "--parent-pid");
    bool interactive = persistent || (argc == 4 && !strcmp(argv[3], "--interactive"));
    if (argc != 3 && !interactive) {
        fprintf(stderr, "Usage: %s socket-path client-uid [--interactive | --parent-pid pid]\n", argv[0]); return 2;
    }
    unsigned long uid_value = parse_number(argv[2]);
    unsigned long pid_value = persistent ? parse_number(argv[4]) : 0;
    char control_path[104];
    if (!uid_value || uid_value > UINT32_MAX || strlen(argv[1]) + 8 >= sizeof(control_path) ||
        argv[1][0] != '/' || (persistent && (pid_value <= 1 || pid_value > INT32_MAX))) return 2;
    if (geteuid() != 0) { fputs("ADMIN_REQUIRED; no USB changes made.\n", stderr); return 77; }
    struct proc_bsdinfo parent = {0};
    pid_t parent_pid = (pid_t)pid_value;
    if (persistent && (proc_pidinfo(parent_pid, PROC_PIDTBSDINFO, 0, &parent, sizeof(parent)) != sizeof(parent) ||
        parent.pbi_uid != (uid_t)uid_value)) return 2;
    /* libusb also uses internal pipes. A disconnected stream during teardown
     * must produce an I/O error, never SIGPIPE termination before USB cleanup. */
    signal(SIGPIPE, SIG_IGN);
    signal(SIGINT, stop); signal(SIGTERM, stop); umask(0077);
    snprintf(control_path, sizeof(control_path), "%s.control", argv[1]);
    int server = create_server(argv[1], (uid_t)uid_value);
    if (server < 0) { perror("stream socket (existing path preserved)"); return 1; }
    int control_server = create_server(control_path, (uid_t)uid_value);
    int result = control_server < 0 ? 1 : 0;
    uvc_context_t *context = NULL;
    if (result) goto done;
    puts("USB_HELPER_LISTENING: GUI startup wait is bounded; interactive sessions have no time limit.");
    unsigned idle_ticks = 0;
    do {
        if (interrupted || !parent_alive(parent_pid, &parent)) break;
        fd_set sockets; FD_ZERO(&sockets); FD_SET(server, &sockets); FD_SET(control_server, &sockets);
        int max_fd = server > control_server ? server : control_server;
        struct timeval wait = {.tv_sec = 1};
        int ready = select(max_fd + 1, &sockets, NULL, NULL, &wait);
        if (ready < 0) { if (errno == EINTR) continue; result = 1; break; }
        if (FD_ISSET(control_server, &sockets)) serve_control(control_server, (uid_t)uid_value, NULL);
        if (interrupted) break;
        if (!FD_ISSET(server, &sockets)) { if (++idle_ticks >= 60 && !persistent) break; continue; }
        client = accept(server, NULL, NULL);
        if (client < 0) continue;
        uid_t peer_uid; gid_t peer_gid;
        uvc_device_t *device = NULL; uvc_device_handle_t *handle = NULL;
        bool streaming = false;
        atomic_store(&failed, false);
        if (getpeereid(client, &peer_uid, &peer_gid) || peer_uid != (uid_t)uid_value) { result = 1; goto session_done; }
        int no_sigpipe = 1; struct timeval timeout = {.tv_sec = 5};
        setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &no_sigpipe, sizeof(no_sigpipe));
        setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
        setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
        /* libuvc's closed context retains kill_handler_thread=1. A new context
         * per connection prevents its next event thread from exiting at once. */
        result = uvc_init(&context, NULL);
        if (!result) result = uvc_find_device(context, &device, 0x21cd, 0x603b, NULL);
        if (!result) result = uvc_open(device, &handle);
        printf("USB_CAPTURE_OPEN result=%d\n", result);
        if (result) goto session_done;
        discover_controls(handle);
        unsigned char hello[16]; little_endian(hello, 2); little_endian(hello + 4, 1280); little_endian(hello + 8, 1024); little_endian(hello + 12, 8);
        if (!packet(0, hello, sizeof(hello)) || !send_control_descriptors()) goto session_done;
        unsigned char start = 0;
        if (recv(client, &start, 1, 0) != 1 || start != 'S') goto session_done;
        uvc_set_button_callback(handle, button_received, NULL);
        uvc_stream_ctrl_t configuration = {0};
        result = uvc_get_stream_ctrl_format_size(handle, &configuration, UVC_FRAME_FORMAT_YUYV, 1280, 1024, 8);
        if (!result) result = uvc_start_streaming(handle, &configuration, frame_received, NULL, 0);
        printf("USB_STREAM_START result=%d controls=%u\n", result, control_count);
        if (result) goto session_done;
        streaming = true;
        puts(interactive ? "USB_HELPER_READY: no session time limit." : "USB_HELPER_READY: diagnostic maximum 300 seconds.");
        for (unsigned tick = 0; !interrupted && !atomic_load(&failed) &&
             parent_alive(parent_pid, &parent) && helper_session_active(interactive, tick, client); tick++) {
            fd_set requests; FD_ZERO(&requests); FD_SET(control_server, &requests);
            struct timeval pause = {.tv_usec = 100000};
            if (select(control_server + 1, &requests, NULL, NULL, &pause) > 0)
                serve_control(control_server, (uid_t)uid_value, handle);
        }
session_done:
        if (streaming) uvc_stop_streaming(handle);
        if (handle) uvc_close(handle);
        if (device) uvc_unref_device(device);
        if (context) { uvc_exit(context); context = NULL; }
        if (result) { char message[128]; int length = snprintf(message, sizeof(message), "DE400 USB helper failed (%d)", result); packet(3, message, (uint32_t)length); }
        close(client); client = -1;
        printf("USB_SESSION_DONE result=%d frames=%u incomplete=%u presses=%u\n", result,
            atomic_load(&frames), atomic_load(&dropped), atomic_load(&presses));
        idle_ticks = 0;
    } while (persistent);
done:
    if (context) uvc_exit(context);
    close(server);
    if (control_server >= 0) close(control_server);
    if (!seteuid((uid_t)uid_value)) { unlink(argv[1]); if (control_server >= 0) unlink(control_path); }
    printf("USB_HELPER_DONE result=%d device_return_requested=1\n", result);
    return result && !persistent ? 1 : 0;
}
