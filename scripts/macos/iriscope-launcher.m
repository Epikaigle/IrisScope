/* Native bundle entry point. Only the USB helper receives administrator rights.
 * No installation, password storage, persistent service or Python dependency.
 */
#import <Cocoa/Cocoa.h>
#import <IOKit/IOKitLib.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/stat.h>
#include <sys/file.h>
#include <fcntl.h>
#include <unistd.h>

static BOOL de400_connected(void) {
    NSMutableDictionary *match = CFBridgingRelease(IOServiceMatching("IOUSBHostDevice"));
    if (!match) return NO;
    match[@"idVendor"] = @0x21cd;
    match[@"idProduct"] = @0x603b;
    io_service_t device = IOServiceGetMatchingService(0, CFBridgingRetain(match));
    if (!device) return NO;
    IOObjectRelease(device);
    return YES;
}
static NSString *shell_quote(NSString *value) {
    return [NSString stringWithFormat:@"'%@'", [value stringByReplacingOccurrencesOfString:@"'" withString:@"'\\''"]];
}
static void alert(NSString *message) {
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    [NSApp activateIgnoringOtherApps:YES];
    NSAlert *notice = [NSAlert new];
    notice.messageText = @"IrisScope";
    notice.informativeText = message;
    [notice runModal];
}
static BOOL launch(NSTask *task, NSError **error) { return [task launchAndReturnError:error]; }
static void quit_helper(NSString *path) {
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return;
    struct sockaddr_un address = {.sun_family = AF_UNIX};
    const char *bytes = [[path stringByAppendingString:@".control"] fileSystemRepresentation];
    if (strlen(bytes) < sizeof(address.sun_path)) {
        strcpy(address.sun_path, bytes);
        if (connect(fd, (struct sockaddr *)&address, sizeof(address)) == 0) {
            int no_sigpipe = 1;
            setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &no_sigpipe, sizeof(no_sigpipe));
            unsigned char quit[12] = {3};
            send(fd, quit, sizeof(quit), 0);
        }
    }
    close(fd);
}
int main(int argc, const char **argv) {
    (void)argc; (void)argv;
    @autoreleasepool {
        umask(0077);
        NSArray<NSString *> *arguments = [[NSProcessInfo processInfo].arguments subarrayWithRange:NSMakeRange(1, [NSProcessInfo processInfo].arguments.count - 1)];
        NSString *macos = [[[NSBundle mainBundle] bundlePath] stringByAppendingPathComponent:@"Contents/MacOS"];
        NSString *gui = [macos stringByAppendingPathComponent:@"IrisScopeGui"];
        NSString *helper = [macos stringByAppendingPathComponent:@"de400-usb-helper"];
        NSFileManager *files = NSFileManager.defaultManager;
        if (![files isExecutableFileAtPath:gui] || ![files isExecutableFileAtPath:helper]) {
            fputs("Incomplete IrisScope bundle: native GUI or USB helper missing.\n", stderr);
            if (!arguments.count) alert(@"L’application est incomplète. Recopie le paquet IrisScope complet.");
            return 1;
        }
        if ([arguments containsObject:@"--check-launcher"]) {
            printf("IrisScope native launcher ready; DE400 connected: %s; no USB device opened.\n", de400_connected() ? "yes" : "no");
            return 0;
        }
        if (geteuid() == 0) { fputs("Launch IrisScope under your normal user account.\n", stderr); return 77; }
        BOOL av_only = [arguments containsObject:@"--avfoundation"];
        NSMutableArray *forwarded = [arguments mutableCopy];
        [forwarded removeObject:@"--avfoundation"];
        BOOL informational = [arguments containsObject:@"--version"] || [arguments containsObject:@"--help"];
        BOOL use_usb = !informational && !av_only;
        NSMutableDictionary *environment = [NSProcessInfo processInfo].environment.mutableCopy;
        NSString *authorized_socket = environment[@"IRISCOPE_DE400_SOCKET"];
        [environment removeObjectForKey:@"IRISCOPE_EXPERIMENTAL_DE400_SOCKET"];
        [environment removeObjectForKey:@"IRISCOPE_DE400_SOCKET"];
        NSTask *admin = nil;
        NSString *socket_path = nil;
        NSString *session = nil;
        NSPipe *admin_output = nil;
        int instance_lock = -1;
        if (!informational) {
            NSString *directory = [NSHomeDirectory() stringByAppendingPathComponent:@"Library/Application Support/IrisScope"];
            if (![files createDirectoryAtPath:directory withIntermediateDirectories:YES attributes:@{NSFilePosixPermissions:@0700} error:NULL]) return 1;
            instance_lock = open([[directory stringByAppendingPathComponent:@"launcher.lock"] fileSystemRepresentation], O_CREAT | O_RDWR | O_NOFOLLOW, 0600);
            if (instance_lock < 0 || flock(instance_lock, LOCK_EX | LOCK_NB)) {
                alert(@"IrisScope est déjà ouverte. Utilise sa fenêtre existante.");
                if (instance_lock >= 0) close(instance_lock);
                return 1;
            }
        }
        if (use_usb && authorized_socket.length) {
            // Developer/testing entry point: a helper already authorized by the
            // caller. It grants no rights; the helper still authenticates the UID.
            if (![authorized_socket isAbsolutePath]) return 2;
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:60];
            while (![files fileExistsAtPath:authorized_socket] && deadline.timeIntervalSinceNow > 0) [NSThread sleepForTimeInterval:0.1];
            if (![files fileExistsAtPath:authorized_socket]) { fputs("Authorized USB socket unavailable.\n", stderr); return 1; }
            environment[@"IRISCOPE_DE400_SOCKET"] = authorized_socket;
        } else if (use_usb) {
            char directory[] = "/private/tmp/iriscope-usb-XXXXXX";
            if (!mkdtemp(directory)) { perror("session directory"); return 1; }
            session = [files stringWithFileSystemRepresentation:directory length:strlen(directory)];
            socket_path = [session stringByAppendingPathComponent:@"camera.sock"];
            NSString *command = [NSString stringWithFormat:@"/usr/bin/nohup %@ %@ %u --parent-pid %d </dev/null >/dev/null 2>&1 & echo $!", shell_quote(helper), shell_quote(socket_path), getuid(), getpid()];
            NSString *script = @"on run argv\ndo shell script (item 1 of argv) with administrator privileges\nend run";
            admin = [NSTask new];
            admin.executableURL = [NSURL fileURLWithPath:@"/usr/bin/osascript"];
            admin.arguments = @[@"-e", script, command];
            admin_output = [NSPipe pipe];
            admin.standardOutput = admin_output;
            admin.standardError = admin_output;
            NSError *error = nil;
            if (!launch(admin, &error)) { alert(error.localizedDescription); [files removeItemAtPath:session error:NULL]; return 1; }
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:120];
            while (admin.running && deadline.timeIntervalSinceNow > 0) [NSThread sleepForTimeInterval:0.1];
            if (admin.running) [admin terminate];
            [admin waitUntilExit];
            NSData *authorization = [admin_output.fileHandleForReading readDataToEndOfFile];
            fwrite(authorization.bytes, 1, authorization.length, stderr);
            deadline = [NSDate dateWithTimeIntervalSinceNow:10];
            while (admin.terminationStatus == 0 && ![files fileExistsAtPath:socket_path] && deadline.timeIntervalSinceNow > 0) [NSThread sleepForTimeInterval:0.1];
            if (![files fileExistsAtPath:socket_path]) {
                [files removeItemAtPath:session error:NULL];
                alert(@"L’accès au bouton USB n’a pas été autorisé ou le composant n’a pas démarré. Relance IrisScope et valide la fenêtre macOS. Le mot de passe reste géré par macOS.");
                return 1;
            }
            environment[@"IRISCOPE_DE400_SOCKET"] = socket_path;
        }
        NSTask *application = [NSTask new];
        application.executableURL = [NSURL fileURLWithPath:gui];
        application.arguments = forwarded;
        application.environment = environment;
        NSError *error = nil;
        BOOL started = launch(application, &error);
        if (started) [application waitUntilExit];
        else alert(error.localizedDescription);
        if (admin) {
            // No session clock: the helper follows this launcher's lifetime.
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:30];
            while ([files fileExistsAtPath:socket_path] && deadline.timeIntervalSinceNow > 0) {
                quit_helper(socket_path);
                [NSThread sleepForTimeInterval:0.2];
            }
            if ([files fileExistsAtPath:socket_path]) fputs("USB cleanup pending; parent exit will stop the helper.\n", stderr);
            [files removeItemAtPath:session error:NULL];
        }
        if (instance_lock >= 0) close(instance_lock);
        return started ? application.terminationStatus : 1;
    }
}
