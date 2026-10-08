/* Root-owned launch-on-demand broker. It only launches our fixed DE400 reader.
 * macOS 13+ authenticates XPC messages against the installed launcher signature.
 * Installation/removal is the only operation requiring an administrator prompt.
 */
#import "usb-service.h"
#include <libproc.h>
#include <signal.h>
#include <errno.h>
#include <sys/file.h>

static int installer_lock(void) {
    if (geteuid() != 0 || !usb_protected_directory(@"/Library") ||
        !usb_protected_directory(@"/Library/Application Support")) return -1;
    int fd = open("/Library/Application Support/.IrisScopeUSB-install.lock", O_CREAT | O_RDWR | O_NOFOLLOW | O_CLOEXEC, 0600);
    struct stat info;
    if (fd < 0) return -1;
    if (fstat(fd, &info) || !S_ISREG(info.st_mode) || info.st_uid != 0 || (info.st_mode & 0022) || flock(fd, LOCK_EX | LOCK_NB)) {
        close(fd); fputs("USB_INSTALL_BUSY_OR_UNSAFE: another installation may be running.\n", stderr); return -1;
    }
    return fd;
}

static int run_tool(NSString *tool, NSArray<NSString *> *arguments) {
    NSTask *task = [NSTask new];
    task.executableURL = [NSURL fileURLWithPath:tool]; task.arguments = arguments;
    task.environment = @{@"PATH": @"/usr/bin:/bin:/usr/sbin:/sbin"};
    NSError *error = nil;
    if (![task launchAndReturnError:&error]) { NSLog(@"%@", error); return 1; }
    [task waitUntilExit]; return task.terminationStatus;
}
static BOOL write_protected(NSData *data, NSString *path, mode_t mode) {
    int fd = open(path.fileSystemRepresentation, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, mode);
    if (fd < 0) return NO;
    size_t offset = 0;
    while (offset < data.length) {
        ssize_t count = write(fd, (const char *)data.bytes + offset, data.length - offset);
        if (count <= 0) { close(fd); unlink(path.fileSystemRepresentation); return NO; }
        offset += (size_t)count;
    }
    BOOL success = !fchmod(fd, mode) && !fsync(fd);
    close(fd); return success;
}
static NSData *plist_data(NSDictionary *value) {
    return [NSPropertyListSerialization dataWithPropertyList:value format:NSPropertyListXMLFormat_v1_0 options:0 error:NULL];
}
static BOOL protected_parents(void) {
    return usb_protected_directory(@"/Library") && usb_protected_directory(@"/Library/Application Support") &&
        usb_protected_directory(@"/Library/LaunchDaemons") && usb_protected_directory(@"/Library/Logs");
}
static int install_service(NSString *bundle) {
    if (geteuid() != 0 || !protected_parents()) return 77;
    NSFileManager *files = NSFileManager.defaultManager;
    NSDictionary *info = [NSDictionary dictionaryWithContentsOfFile:[bundle stringByAppendingPathComponent:@"Contents/Info.plist"]];
    NSDictionary *manifest = usb_bundle_manifest(bundle);
    if (![info[@"CFBundleIdentifier"] isEqual:@"app.iriscope.IrisScope"] || !manifest || !usb_valid_signature(bundle, nil)) {
        fputs("USB_INSTALL_REFUSED: invalid application signature or identity.\n", stderr); return 1;
    }
    if (![files fileExistsAtPath:IRISCOPE_USB_ROOT] &&
        (mkdir(IRISCOPE_USB_ROOT.fileSystemRepresentation, 0755) || chmod(IRISCOPE_USB_ROOT.fileSystemRepresentation, 0755))) return 1;
    if (!usb_protected_directory(IRISCOPE_USB_ROOT)) return 1;
    NSString *stage = [IRISCOPE_USB_ROOT stringByAppendingPathComponent:[@"stage-" stringByAppendingString:NSUUID.UUID.UUIDString]];
    if (mkdir(stage.fileSystemRepresentation, 0755) || chmod(stage.fileSystemRepresentation, 0755)) return 1;
    NSString *macos = [bundle stringByAppendingPathComponent:@"Contents/MacOS"];
    BOOL ready = YES;
    for (NSString *name in @[@"iriscope-usb-service", @"de400-usb-helper"]) {
        NSData *data = usb_read_regular([macos stringByAppendingPathComponent:name], NO);
        NSString *expected = manifest[[name isEqual:@"iriscope-usb-service"] ? @"brokerSHA256" : @"helperSHA256"];
        ready = ready && [usb_hash(data) isEqual:expected] && write_protected(data, [stage stringByAppendingPathComponent:name], 0755);
    }
    ready = ready && usb_valid_signature([stage stringByAppendingPathComponent:@"iriscope-usb-service"], manifest[@"brokerRequirement"]) &&
        usb_valid_signature([stage stringByAppendingPathComponent:@"de400-usb-helper"], nil) &&
        write_protected(plist_data(manifest), [stage stringByAppendingPathComponent:@"manifest.plist"], 0644);
    if (!ready) { [files removeItemAtPath:stage error:NULL]; return 1; }
    NSString *log = @"/Library/Logs/IrisScopeUSB.log";
    int logfd = open(log.fileSystemRepresentation, O_WRONLY | O_APPEND | O_CREAT | O_NOFOLLOW | O_CLOEXEC, 0600);
    struct stat logInfo;
    if (logfd < 0 || fstat(logfd, &logInfo) || !S_ISREG(logInfo.st_mode) || logInfo.st_uid != 0 || (logInfo.st_mode & 0022)) {
        if (logfd >= 0) close(logfd);
        [files removeItemAtPath:stage error:NULL]; return 1;
    }
    close(logfd);
    NSDictionary *job = @{@"Label": IRISCOPE_USB_LABEL,
        @"ProgramArguments": @[IRISCOPE_USB_CURRENT @"/iriscope-usb-service"],
        @"MachServices": @{IRISCOPE_USB_LABEL: @YES}, @"ProcessType": @"Interactive",
        @"StandardOutPath": log, @"StandardErrorPath": log};
    NSString *jobStage = [IRISCOPE_USB_ROOT stringByAppendingPathComponent:@"job-staging.plist"];
    unlink(jobStage.fileSystemRepresentation);
    if (!write_protected(plist_data(job), jobStage, 0644)) { [files removeItemAtPath:stage error:NULL]; return 1; }
    NSString *previous = IRISCOPE_USB_ROOT @"/previous";
    BOOL existed = usb_protected_directory(IRISCOPE_USB_CURRENT);
    NSData *oldJob = usb_read_regular(IRISCOPE_USB_PLIST, YES);
    if (([files fileExistsAtPath:IRISCOPE_USB_CURRENT] && (!existed || !usb_installed_manifest())) ||
        ([files fileExistsAtPath:IRISCOPE_USB_PLIST] && !oldJob) ||
        ([files fileExistsAtPath:previous] && ![files removeItemAtPath:previous error:NULL])) {
        [files removeItemAtPath:stage error:NULL]; unlink(jobStage.fileSystemRepresentation); return 1;
    }
    int stopped = run_tool(@"/bin/launchctl", @[@"bootout", @"system/" IRISCOPE_USB_LABEL]);
    if ((stopped && stopped != 3) ||
        (existed && rename(IRISCOPE_USB_CURRENT.fileSystemRepresentation, previous.fileSystemRepresentation))) {
        if (!stopped && oldJob) run_tool(@"/bin/launchctl", @[@"bootstrap", @"system", IRISCOPE_USB_PLIST]);
        [files removeItemAtPath:stage error:NULL]; unlink(jobStage.fileSystemRepresentation); return 1;
    }
    int result = rename(stage.fileSystemRepresentation, IRISCOPE_USB_CURRENT.fileSystemRepresentation);
    if (!result) result = rename(jobStage.fileSystemRepresentation, IRISCOPE_USB_PLIST.fileSystemRepresentation);
    if (!result) result = run_tool(@"/bin/launchctl", @[@"bootstrap", @"system", IRISCOPE_USB_PLIST]);
    if (result) {
        [files removeItemAtPath:IRISCOPE_USB_CURRENT error:NULL];
        unlink(IRISCOPE_USB_PLIST.fileSystemRepresentation);
        if (existed) {
            rename(previous.fileSystemRepresentation, IRISCOPE_USB_CURRENT.fileSystemRepresentation);
            if (oldJob && write_protected(oldJob, IRISCOPE_USB_PLIST, 0644))
                run_tool(@"/bin/launchctl", @[@"bootstrap", @"system", IRISCOPE_USB_PLIST]);
        }
        [files removeItemAtPath:stage error:NULL];
        unlink(jobStage.fileSystemRepresentation);
        fputs("USB_INSTALL_FAILED: previous installation restored when available.\n", stderr); return 1;
    }
    [files removeItemAtPath:previous error:NULL];
    puts("USB_SERVICE_INSTALLED: root-owned, signed, launch-on-demand; no password stored.");
    return 0;
}
static int uninstall_service(void) {
    if (geteuid() != 0 || !protected_parents() || !usb_installed_manifest()) return 77;
    run_tool(@"/bin/launchctl", @[@"bootout", @"system/" IRISCOPE_USB_LABEL]);
    if (unlink(IRISCOPE_USB_PLIST.fileSystemRepresentation) && errno != ENOENT) return 1;
    NSError *error = nil;
    if (![NSFileManager.defaultManager removeItemAtPath:IRISCOPE_USB_ROOT error:&error]) { NSLog(@"%@", error); return 1; }
    puts("USB_SERVICE_REMOVED: application and patient files preserved."); return 0;
}

@class USBClient;
@interface USBListener : NSObject <NSXPCListenerDelegate>
@property NSDictionary *manifest;
@property USBClient *activeClient;
@property dispatch_queue_t queue;
@end
@interface USBClient : NSObject <IrisScopeUSBService>
@property (weak) USBListener *owner;
@property uid_t uid;
@property pid_t pid;
@property NSTask *reader;
@property NSString *directory;
- (void)finish;
@end
@implementation USBClient
- (void)statusWithReply:(void (^)(NSString *))reply { reply(usb_revision(self.owner.manifest)); }
- (void)finish {
    if (self.reader.running) {
        /* The launcher already requested shutdown; a dead parent is also
         * monitored by the reader. Let its USB teardown finish before signals. */
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:2];
        while (self.reader.running && deadline.timeIntervalSinceNow > 0) [NSThread sleepForTimeInterval:0.05];
        if (self.reader.running) {
            [self.reader terminate];
            deadline = [NSDate dateWithTimeIntervalSinceNow:8];
            while (self.reader.running && deadline.timeIntervalSinceNow > 0) [NSThread sleepForTimeInterval:0.05];
        }
        if (self.reader.running) {
            NSLog(@"USB_SERVICE_FORCED_READER_STOP uid=%u", self.uid);
            kill(self.reader.processIdentifier, SIGKILL);
        }
        [self.reader waitUntilExit];
        NSLog(@"USB_SERVICE_READER_EXIT reason=%ld status=%d", (long)self.reader.terminationReason, self.reader.terminationStatus);
    }
    self.reader = nil;
    /* The existing reader unlinks its sockets with the user's privileges.
     * Never recursively remove a user-owned session directory as root. */
    if (self.directory) rmdir(self.directory.fileSystemRepresentation);
    self.directory = nil;
    if (self.owner.activeClient == self) self.owner.activeClient = nil;
    NSLog(@"USB_SERVICE_SESSION_CLOSED uid=%u", self.uid);
}
- (void)startSessionWithReply:(void (^)(NSString *, NSString *))reply {
    USBListener *owner = self.owner;
    dispatch_async(owner.queue, ^{
        if (owner.activeClient && owner.activeClient != self) {
            reply(@"", @"Le DE400 est déjà utilisé par une autre session IrisScope."); return;
        }
        if (self.reader.running) {
            reply([self.directory stringByAppendingPathComponent:@"camera.sock"], @""); return;
        }
        struct proc_bsdinfo parent;
        if (proc_pidinfo(self.pid, PROC_PIDTBSDINFO, 0, &parent, sizeof(parent)) != sizeof(parent) || parent.pbi_uid != self.uid) {
            reply(@"", @"La session utilisateur n’est plus disponible."); return;
        }
        char template[] = "/private/tmp/iriscope-usb-XXXXXX";
        char *directory = mkdtemp(template);
        if (!directory || chown(directory, self.uid, parent.pbi_gid)) {
            if (directory) rmdir(directory);
            reply(@"", @"Impossible de préparer la réception USB."); return;
        }
        self.directory = [NSString stringWithUTF8String:directory];
        NSString *socket = [self.directory stringByAppendingPathComponent:@"camera.sock"];
        self.reader = [NSTask new];
        self.reader.executableURL = [NSURL fileURLWithPath:IRISCOPE_USB_CURRENT @"/de400-usb-helper"];
        self.reader.arguments = @[socket, [NSString stringWithFormat:@"%u", self.uid], @"--parent-pid", [NSString stringWithFormat:@"%d", self.pid]];
        self.reader.environment = @{@"PATH": @"/usr/bin:/bin"};
        self.reader.standardInput = [NSFileHandle fileHandleWithNullDevice];
        NSError *error = nil;
        if (![self.reader launchAndReturnError:&error]) {
            [self finish]; reply(@"", error.localizedDescription); return;
        }
        owner.activeClient = self;
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:10];
        while (self.reader.running && deadline.timeIntervalSinceNow > 0 &&
               ![NSFileManager.defaultManager fileExistsAtPath:socket]) [NSThread sleepForTimeInterval:0.05];
        if (![NSFileManager.defaultManager fileExistsAtPath:socket]) {
            [self finish]; reply(@"", @"Le composant USB n’a pas démarré. Répare son installation."); return;
        }
        NSLog(@"USB_SERVICE_SESSION_READY uid=%u parent=%d", self.uid, self.pid);
        reply(socket, @"");
    });
}
@end
@implementation USBListener
- (BOOL)listener:(NSXPCListener *)listener shouldAcceptNewConnection:(NSXPCConnection *)connection {
    (void)listener;
    if (connection.effectiveUserIdentifier == 0 || connection.processIdentifier <= 1) return NO;
    /* Enforced by XPC on every incoming message, avoiding PID-only validation. */
    [connection setCodeSigningRequirement:self.manifest[@"launcherRequirement"]];
    USBClient *client = [USBClient new]; client.owner = self;
    client.uid = connection.effectiveUserIdentifier; client.pid = connection.processIdentifier;
    connection.exportedInterface = [NSXPCInterface interfaceWithProtocol:@protocol(IrisScopeUSBService)];
    connection.exportedObject = client;
    __weak USBListener *weakSelf = self;
    connection.invalidationHandler = ^{
        USBListener *owner = weakSelf;
        if (owner) dispatch_async(owner.queue, ^{ [client finish]; });
    };
    [connection resume]; return YES;
}
@end
int main(int argc, const char **argv) {
    @autoreleasepool {
        umask(0077);
        if ((argc == 3 && !strcmp(argv[1], "--install")) || (argc == 2 && !strcmp(argv[1], "--uninstall"))) {
            int lock = installer_lock();
            if (lock < 0) return 77;
            int result = argc == 3 ? install_service([[NSString stringWithUTF8String:argv[2]] stringByResolvingSymlinksInPath]) : uninstall_service();
            close(lock); return result;
        }
        if (argc != 1 || geteuid() != 0) { fputs("Root service or installer only.\n", stderr); return 77; }
        NSDictionary *manifest = usb_installed_manifest();
        if (!manifest || ![usb_hash(usb_read_regular(IRISCOPE_USB_CURRENT @"/iriscope-usb-service", YES)) isEqual:manifest[@"brokerSHA256"]] ||
            ![usb_hash(usb_read_regular(IRISCOPE_USB_CURRENT @"/de400-usb-helper", YES)) isEqual:manifest[@"helperSHA256"]]) return 1;
        if (@available(macOS 13.0, *)) {
            USBListener *delegate = [USBListener new]; delegate.manifest = manifest;
            delegate.queue = dispatch_queue_create("app.iriscope.usb-session", DISPATCH_QUEUE_SERIAL);
            NSXPCListener *listener = [[NSXPCListener alloc] initWithMachServiceName:IRISCOPE_USB_LABEL];
            listener.delegate = delegate;
            signal(SIGTERM, SIG_IGN);
            dispatch_source_t termination = dispatch_source_create(DISPATCH_SOURCE_TYPE_SIGNAL, SIGTERM, 0, delegate.queue);
            dispatch_source_set_event_handler(termination, ^{ [delegate.activeClient finish]; exit(0); });
            dispatch_resume(termination);
            [listener resume];
            NSLog(@"USB_SERVICE_READY revision=%@; idle until IrisScope connects", usb_revision(manifest));
            [[NSRunLoop currentRunLoop] run];
            return 0;
        }
        return 77;
    }
}
