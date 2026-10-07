/* Real Unix sockets exercise lifetime and cleanup without opening USB. */
#include "helper-session.h"
#include <assert.h>
#include <limits.h>
#include <stdio.h>
#include <unistd.h>

int main(void) {
    int sockets[2];
    assert(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets) == 0);
    assert(helper_session_active(true, 36000, sockets[0]));
    assert(helper_session_active(true, 864000, sockets[0]));
    assert(helper_session_active(true, UINT_MAX, sockets[0]));
    assert(helper_session_active(false, 2999, sockets[0]));
    assert(!helper_session_active(false, 3000, sockets[0]));
    assert(send(sockets[1], "?", 1, 0) == 1);
    assert(!helper_session_active(true, 0, sockets[0]));
    char command;
    assert(recv(sockets[0], &command, 1, 0) == 1);
    close(sockets[1]);
    assert(!helper_session_active(true, 0, sockets[0]));
    close(sockets[0]);
    assert(!helper_session_active(true, 0, -1));
    puts("PASS: no interactive expiry; diagnostic expiry, peer closure and invalid IPC stop safely.");
    return 0;
}
