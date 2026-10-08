/* Fixed, signed client/service protocol. No caller-supplied command or path. */
#import <Foundation/Foundation.h>
#import <Security/Security.h>
#include <CommonCrypto/CommonDigest.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define IRISCOPE_USB_LABEL @"app.iriscope.usb-service"
#define IRISCOPE_USB_ROOT @"/Library/Application Support/IrisScopeUSB"
#define IRISCOPE_USB_CURRENT IRISCOPE_USB_ROOT @"/current"
#define IRISCOPE_USB_PLIST @"/Library/LaunchDaemons/app.iriscope.usb-service.plist"

@protocol IrisScopeUSBService
- (void)statusWithReply:(void (^)(NSString *revision))reply;
- (void)startSessionWithReply:(void (^)(NSString *socketPath, NSString *error))reply;
@end

static inline NSData *usb_read_regular(NSString *path, BOOL protectedFile) {
    /* Nonblocking open lets fstat reject a FIFO instead of waiting for a writer. */
    int fd = open(path.fileSystemRepresentation, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK);
    if (fd < 0) return nil;
    struct stat info;
    if (fstat(fd, &info) || !S_ISREG(info.st_mode) || info.st_size <= 0 ||
        info.st_size > 16 * 1024 * 1024 ||
        (protectedFile && (info.st_uid != 0 || (info.st_mode & 0022)))) {
        close(fd); return nil;
    }
    NSMutableData *data = [NSMutableData dataWithLength:(NSUInteger)info.st_size];
    size_t offset = 0;
    while (offset < data.length) {
        ssize_t count = read(fd, (char *)data.mutableBytes + offset, data.length - offset);
        if (count <= 0) { close(fd); return nil; }
        offset += (size_t)count;
    }
    close(fd);
    return data;
}
static inline NSString *usb_hash(NSData *data) {
    if (!data) return nil;
    unsigned char digest[CC_SHA256_DIGEST_LENGTH];
    CC_SHA256(data.bytes, (CC_LONG)data.length, digest);
    NSMutableString *text = [NSMutableString stringWithCapacity:64];
    for (unsigned i = 0; i < sizeof(digest); i++) [text appendFormat:@"%02x", digest[i]];
    return text;
}
static inline NSString *usb_requirement(NSString *path) {
    SecStaticCodeRef code = NULL; SecRequirementRef requirement = NULL; CFStringRef text = NULL;
    OSStatus result = SecStaticCodeCreateWithPath((__bridge CFURLRef)[NSURL fileURLWithPath:path], 0, &code);
    if (!result) result = SecCodeCopyDesignatedRequirement(code, 0, &requirement);
    if (!result) result = SecRequirementCopyString(requirement, 0, &text);
    if (requirement) CFRelease(requirement);
    if (code) CFRelease(code);
    return !result ? CFBridgingRelease(text) : nil;
}
static inline BOOL usb_valid_signature(NSString *path, NSString *requirementText) {
    SecStaticCodeRef code = NULL; SecRequirementRef requirement = NULL;
    OSStatus result = SecStaticCodeCreateWithPath((__bridge CFURLRef)[NSURL fileURLWithPath:path], 0, &code);
    if (!result && requirementText)
        result = SecRequirementCreateWithString((__bridge CFStringRef)requirementText, 0, &requirement);
    if (!result) result = SecStaticCodeCheckValidity(code, kSecCSStrictValidate | kSecCSCheckNestedCode, requirement);
    if (requirement) CFRelease(requirement);
    if (code) CFRelease(code);
    return result == errSecSuccess;
}
static inline BOOL usb_protected_directory(NSString *path) {
    struct stat info;
    return !lstat(path.fileSystemRepresentation, &info) && S_ISDIR(info.st_mode) &&
        info.st_uid == 0 && !(info.st_mode & 0022);
}
static inline NSDictionary *usb_installed_manifest(void) {
    if (!usb_protected_directory(IRISCOPE_USB_ROOT) || !usb_protected_directory(IRISCOPE_USB_CURRENT)) return nil;
    NSData *data = usb_read_regular(IRISCOPE_USB_CURRENT @"/manifest.plist", YES);
    if (!data) return nil;
    id value = [NSPropertyListSerialization propertyListWithData:data options:NSPropertyListImmutable format:NULL error:NULL];
    return [value isKindOfClass:NSDictionary.class] && [value[@"format"] isEqual:@1] ? value : nil;
}
static inline NSDictionary *usb_bundle_manifest(NSString *bundle) {
    NSString *macos = [bundle stringByAppendingPathComponent:@"Contents/MacOS"];
    NSString *launcher = usb_requirement(bundle);
    NSString *broker = usb_requirement([macos stringByAppendingPathComponent:@"iriscope-usb-service"]);
    NSString *brokerHash = usb_hash(usb_read_regular([macos stringByAppendingPathComponent:@"iriscope-usb-service"], NO));
    NSString *helperHash = usb_hash(usb_read_regular([macos stringByAppendingPathComponent:@"de400-usb-helper"], NO));
    if (!launcher || !broker || !brokerHash || !helperHash) return nil;
    return @{@"format": @1, @"launcherRequirement": launcher, @"brokerRequirement": broker,
             @"brokerSHA256": brokerHash, @"helperSHA256": helperHash};
}
static inline NSString *usb_revision(NSDictionary *manifest) {
    if (!manifest) return @"";
    NSString *text = [NSString stringWithFormat:@"%@|%@|%@", manifest[@"launcherRequirement"],
                      manifest[@"brokerSHA256"], manifest[@"helperSHA256"]];
    return usb_hash([text dataUsingEncoding:NSUTF8StringEncoding]);
}
