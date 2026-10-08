/* Launcher-side installation and authenticated XPC; never saves a password. */
#import "usb-service.h"

static inline BOOL usb_valid_session_path(NSString *socket) {
    /* Validate lexically: stringByStandardizingPath rewrites an existing
     * /private/tmp path to /tmp on macOS, unlike the same nonexistent path. */
    NSArray<NSString *> *parts = [socket componentsSeparatedByString:@"/"];
    if (parts.count != 5 || ![parts[0] isEqual:@""] || ![parts[1] isEqual:@"private"] ||
        ![parts[2] isEqual:@"tmp"] || ![parts[4] isEqual:@"camera.sock"]) return NO;
    NSString *name = parts[3];
    NSString *prefix = @"iriscope-usb-";
    if (![name hasPrefix:prefix] || name.length != prefix.length + 6) return NO;
    NSCharacterSet *allowed = [NSCharacterSet characterSetWithCharactersInString:@"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"];
    return [[name substringFromIndex:prefix.length] rangeOfCharacterFromSet:allowed.invertedSet].location == NSNotFound;
}

static inline NSXPCConnection *usb_service_connection(NSDictionary *manifest) {
    NSXPCConnection *connection = [[NSXPCConnection alloc] initWithMachServiceName:IRISCOPE_USB_LABEL options:NSXPCConnectionPrivileged];
    connection.remoteObjectInterface = [NSXPCInterface interfaceWithProtocol:@protocol(IrisScopeUSBService)];
    [connection setCodeSigningRequirement:manifest[@"brokerRequirement"]];
    [connection resume]; return connection;
}
static inline NSString *usb_service_status(NSXPCConnection *connection) {
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    __block NSString *revision = nil;
    id<IrisScopeUSBService> proxy = [connection remoteObjectProxyWithErrorHandler:^(NSError *error) {
        fprintf(stderr, "USB_SERVICE_UNAVAILABLE: %s\n", error.localizedDescription.UTF8String);
        dispatch_semaphore_signal(ready);
    }];
    [proxy statusWithReply:^(NSString *value) { revision = value; dispatch_semaphore_signal(ready); }];
    if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC))) return nil;
    return revision;
}
static inline NSString *usb_service_session(NSXPCConnection *connection, NSString **errorMessage) {
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    __block NSString *socket = nil; __block NSString *failure = nil;
    id<IrisScopeUSBService> proxy = [connection remoteObjectProxyWithErrorHandler:^(NSError *error) {
        failure = error.localizedDescription; dispatch_semaphore_signal(ready);
    }];
    [proxy startSessionWithReply:^(NSString *value, NSString *error) {
        socket = value; failure = error; dispatch_semaphore_signal(ready);
    }];
    if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 15 * NSEC_PER_SEC)))
        failure = @"Le composant USB ne répond pas. Ferme puis relance IrisScope.";
    if (!socket.length) { if (errorMessage) *errorMessage = failure; return nil; }
    /* Only accept the session paths produced by our authenticated root broker. */
    if (!usb_valid_session_path(socket)) {
        if (errorMessage) *errorMessage = @"Réponse USB invalide.";
        return nil;
    }
    return socket;
}
