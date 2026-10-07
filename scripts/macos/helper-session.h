#ifndef IRISCOPE_HELPER_SESSION_H
#define IRISCOPE_HELPER_SESSION_H

#include <errno.h>
#include <stdbool.h>
#include <sys/socket.h>

/* Only the diagnostic mode has a duration limit. Interactive sessions end
 * when their GUI disconnects, including when an unplugged camera stops frames.
 */
static inline bool helper_session_active(bool interactive, unsigned tick, int socket_fd) {
    if (!interactive && tick >= 3000) return false;
    unsigned char unexpected_command;
    ssize_t count = recv(socket_fd, &unexpected_command, 1, MSG_PEEK | MSG_DONTWAIT);
    return count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR);
}

#endif
