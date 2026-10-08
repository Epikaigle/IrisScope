/* Real file-boundary checks; optional rejection test against an installed service.
 * This program is intentionally not an authorized IrisScope launcher.
 */
#import "usb-service-client.h"
#include <assert.h>

int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc == 2 && !strcmp(argv[1], "--rejected-client")) {
            NSDictionary *manifest = usb_installed_manifest();
            if (!manifest) return 1;
            NSXPCConnection *connection = usb_service_connection(manifest);
            dispatch_semaphore_t ready = dispatch_semaphore_create(0);
            __block BOOL rejected = NO, accepted = NO;
            id<IrisScopeUSBService> proxy = [connection remoteObjectProxyWithErrorHandler:^(NSError *error) {
                fprintf(stderr, "Rejected connection: %s %ld\n", error.domain.UTF8String, (long)error.code);
                rejected = [error.domain isEqual:NSCocoaErrorDomain] &&
                    (error.code == NSXPCConnectionInvalid || error.code == NSXPCConnectionInterrupted);
                dispatch_semaphore_signal(ready);
            }];
            [proxy startSessionWithReply:^(NSString *socket, NSString *error) {
                (void)socket; (void)error; accepted = YES; dispatch_semaphore_signal(ready);
            }];
            BOOL timedOut = dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC)) != 0;
            [connection invalidate];
            if (!timedOut && rejected && !accepted) { puts("PASS: untrusted client cannot request a privileged USB session."); return 0; }
            fputs("FAIL: unexpected client authorization result.\n", stderr); return 1;
        }
        if (argc != 1 || geteuid() == 0) return 77;
        char sessionTemplate[] = "/private/tmp/iriscope-usb-XXXXXX";
        char *sessionDirectory = mkdtemp(sessionTemplate); assert(sessionDirectory);
        NSString *socket = [[NSString stringWithUTF8String:sessionDirectory] stringByAppendingPathComponent:@"camera.sock"];
        assert([[@"socket placeholder" dataUsingEncoding:NSUTF8StringEncoding] writeToFile:socket atomically:NO]);
        assert(usb_valid_session_path(socket));
        for (NSString *invalid in @[@"/tmp/iriscope-usb-ABC123/camera.sock", @"/private/tmp/iriscope-usb-ABC123/../camera.sock",
             @"/private/tmp/iriscope-usb-ABC123/camera.sock/", @"/private/tmp/iriscope-usb-ABC123/camera.sock.control",
             @"/private/tmp/iriscope-usb-ABC12./camera.sock", @"/private/tmp/iriscope-usb-ABC123/other", @"", @"relative"])
            assert(!usb_valid_session_path(invalid));
        unlink(socket.fileSystemRepresentation); rmdir(sessionDirectory);
        char template[] = "/private/tmp/iriscope-file-test-XXXXXX";
        char *directory = mkdtemp(template); assert(directory);
        NSString *base = [NSString stringWithUTF8String:directory];
        NSString *file = [base stringByAppendingPathComponent:@"data"];
        NSData *payload = [@"trusted source bytes" dataUsingEncoding:NSUTF8StringEncoding];
        assert([payload writeToFile:file atomically:NO]);
        assert([usb_read_regular(file, NO) isEqual:payload]);
        assert(!usb_read_regular(file, YES)); // Normal-user file cannot be a root manifest.
        NSString *link = [base stringByAppendingPathComponent:@"link"];
        assert(!symlink(file.fileSystemRepresentation, link.fileSystemRepresentation));
        assert(!usb_read_regular(link, NO));
        assert(!usb_read_regular(base, NO));
        assert(!usb_protected_directory(base));
        NSString *fifo = [base stringByAppendingPathComponent:@"fifo"];
        assert(!mkfifo(fifo.fileSystemRepresentation, 0600));
        assert(!usb_read_regular(fifo, NO));
        int fd = open(file.fileSystemRepresentation, O_WRONLY);
        assert(fd >= 0 && !ftruncate(fd, 16 * 1024 * 1024 + 1)); close(fd);
        assert(!usb_read_regular(file, NO));
        unlink(fifo.fileSystemRepresentation); unlink(link.fileSystemRepresentation); unlink(file.fileSystemRepresentation); rmdir(directory);
        puts("PASS: existing USB session paths accepted; traversal, FIFOs, symlinks, oversized files and user-owned protected manifests refused.");
        return 0;
    }
}
