/* Temporary, opt-in DE400 USB capture test. Requires root only for --capture.
 * Targets 21cd:603b, serial VTU603EB, location 14100000 on the test Mac.
 * --share-video tests returning the video driver while retaining the button interface.
 * No firmware/configuration writes, driver installation, or image files.
 * Always attempts to return the captured device to macOS, including on SIGINT/SIGTERM.
 */
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <IOKit/IOCFPlugIn.h>
#include <IOKit/usb/IOUSBLib.h>
#include <IOKit/usb/USB.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static volatile sig_atomic_t stopped;
static IOUSBDeviceInterface500 **device;
static IOUSBInterfaceInterface220 **interface;
static bool captured, interface_open;
static bool seize_interface;
static UInt8 buffer[64], pipe_ref;
static unsigned packets;
static void stop(int signal) { (void)signal; stopped=1; }
static int integer(io_service_t service, CFStringRef key) {
    int value=-1;CFTypeRef prop=IORegistryEntryCreateCFProperty(service,key,kCFAllocatorDefault,0);
    if(prop){if(CFGetTypeID(prop)==CFNumberGetTypeID())CFNumberGetValue(prop,kCFNumberIntType,&value);CFRelease(prop);}return value;
}
static bool exact_device(io_service_t service) {
    if(integer(service,CFSTR("idVendor"))!=0x21cd || integer(service,CFSTR("idProduct"))!=0x603b || integer(service,CFSTR("locationID"))!=0x14100000)return false;
    CFTypeRef serial=IORegistryEntryCreateCFProperty(service,CFSTR("USB Serial Number"),kCFAllocatorDefault,0);
    bool match=serial&&CFGetTypeID(serial)==CFStringGetTypeID()&&CFEqual(serial,CFSTR("VTU603EB"));if(serial)CFRelease(serial);return match;
}
static bool open_device(void) {
    io_iterator_t iterator=0;if(IOServiceGetMatchingServices(kIOMainPortDefault,IOServiceMatching(kIOUSBDeviceClassName),&iterator))return false;
    io_service_t service;bool found=false;
    while((service=IOIteratorNext(iterator))) {
        if(exact_device(service)){IOCFPlugInInterface **plugin=NULL;SInt32 score=0;IOReturn r=IOCreatePlugInInterfaceForService(service,kIOUSBDeviceUserClientTypeID,kIOCFPlugInInterfaceID,&plugin,&score);
            if(!r&&plugin){HRESULT q=(*plugin)->QueryInterface(plugin,CFUUIDGetUUIDBytes(kIOUSBDeviceInterfaceID500),(LPVOID*)&device);(*plugin)->Release(plugin);found=!q&&device;}
        }
        IOObjectRelease(service);if(found)break;
    }IOObjectRelease(iterator);return found;
}
static void cleanup(void) {
    stopped=1;
    if(interface){if(interface_open){if(pipe_ref)(*interface)->AbortPipe(interface,pipe_ref);(*interface)->USBInterfaceClose(interface);}(*interface)->Release(interface);interface=NULL;}
    if(device){if(captured){IOReturn r=(*device)->USBDeviceReEnumerate(device,kUSBReEnumerateReleaseDeviceMask);printf("DEVICE_RETURN_TO_MACOS result=0x%08x\n",r);captured=false;}(*device)->USBDeviceClose(device);(*device)->Release(device);device=NULL;}
}
static bool submit(void);
static void completed(void *context, IOReturn result, void *size) {
    (void)context;UInt32 count=(UInt32)(uintptr_t)size;
    printf("STATUS_PACKET result=0x%08x size=%u bytes=",result,count);for(UInt32 i=0;i<count&&i<sizeof(buffer);i++)printf("%02x",buffer[i]);puts("");
    if(!result){packets++;if(count==4&&buffer[0]==2&&buffer[1]==1&&buffer[2]==0)printf("DE400_%s\n",buffer[3]==1?"PRESSED":buffer[3]==0?"RELEASED":"UNKNOWN");}
    if(!stopped&&!result)submit();else stopped=1;
}
static bool submit(void) { IOReturn r=(*interface)->ReadPipeAsync(interface,pipe_ref,buffer,sizeof(buffer),completed,NULL);if(r){printf("READ_SUBMIT result=0x%08x\n",r);return false;}return true; }
static bool claim_button(void) {
    IOUSBFindInterfaceRequest filter={kIOUSBFindInterfaceDontCare,kIOUSBFindInterfaceDontCare,kIOUSBFindInterfaceDontCare,kIOUSBFindInterfaceDontCare};io_iterator_t iterator=0;
    if((*device)->CreateInterfaceIterator(device,&filter,&iterator))return false;
    io_service_t service;bool success=false;
    while((service=IOIteratorNext(iterator))) {
        if(integer(service,CFSTR("bInterfaceNumber"))==0){IOCFPlugInInterface **plugin=NULL;SInt32 score=0;IOReturn r=IOCreatePlugInInterfaceForService(service,kIOUSBInterfaceUserClientTypeID,kIOCFPlugInInterfaceID,&plugin,&score);
            if(!r&&plugin){(*plugin)->QueryInterface(plugin,CFUUIDGetUUIDBytes(kIOUSBInterfaceInterfaceID220),(LPVOID*)&interface);(*plugin)->Release(plugin);}
            if(interface){r=seize_interface?(*interface)->USBInterfaceOpenSeize(interface):(*interface)->USBInterfaceOpen(interface);printf("BUTTON_INTERFACE_OPEN result=0x%08x seize=%d\n",r,seize_interface);if(!r){interface_open=true;UInt8 count=0;(*interface)->GetNumEndpoints(interface,&count);
                for(UInt8 pipe=1;pipe<=count;pipe++){UInt8 direction=0,number=0,type=0,interval=0;UInt16 packet_size=0;r=(*interface)->GetPipeProperties(interface,pipe,&direction,&number,&type,&packet_size,&interval);
                    printf("PIPE ref=%u address=%02x type=%u max_packet=%u result=0x%08x\n",pipe,number|(direction==kUSBIn?128:0),type,packet_size,r);
                    if(!r&&direction==kUSBIn&&number==1&&type==kUSBInterrupt&&packet_size<=sizeof(buffer)){pipe_ref=pipe;success=true;}
                }
            }}
        }IOObjectRelease(service);if(success||interface)break;
    }IOObjectRelease(iterator);return success;
}
int main(int argc,char **argv) {
    setbuf(stdout,NULL);bool share_video=argc==2&&!strcmp(argv[1],"--share-video");bool capture=share_video||(argc==2&&!strcmp(argv[1],"--capture"));
    bool describe=argc==2&&!strcmp(argv[1],"--describe");
    seize_interface=argc==2&&!strcmp(argv[1],"--seize-interface");
    if(!capture&&!seize_interface&&!describe&&argc!=1){fprintf(stderr,"Usage: %s [--capture | --share-video | --seize-interface | --describe]\n",argv[0]);return 2;}
    if(capture&&geteuid()!=0){fputs("ADMIN_REQUIRED: macOS USB device capture requires root or an authorized VM entitlement. No device changes made.\n",stderr);return 77;}
    signal(SIGINT,stop);signal(SIGTERM,stop);atexit(cleanup);
    if(!open_device()){fputs("Exact test DE400 not found; no other devices are touched.\n",stderr);return 1;}
    IOReturn r=(*device)->USBDeviceOpen(device);printf("DEVICE_OPEN result=0x%08x capture_requested=%d\n",r,capture);
    if(describe){IOUSBConfigurationDescriptorPtr descriptor=NULL;r=(*device)->GetConfigurationDescriptorPtr(device,0,&descriptor);printf("CACHED_CONFIG result=0x%08x bytes=",r);if(!r&&descriptor){unsigned length=USBToHostWord(descriptor->wTotalLength);for(unsigned i=0;i<length;i++)printf("%02x",((unsigned char*)descriptor)[i]);}puts("");
        unsigned char raw[4096]={0};IOUSBDevRequest request={.bmRequestType=0x80,.bRequest=kUSBRqGetDescriptor,.wValue=0x0200,.wIndex=0,.wLength=sizeof(raw),.pData=raw};r=(*device)->DeviceRequest(device,&request);printf("RAW_CONFIG result=0x%08x size=%u bytes=",r,request.wLenDone);if(!r)for(unsigned i=0;i<request.wLenDone&&i<sizeof(raw);i++)printf("%02x",raw[i]);puts("");return r?1:0;}
    if(capture){r=(*device)->USBDeviceReEnumerate(device,kUSBReEnumerateCaptureDeviceMask);printf("DEVICE_CAPTURE result=0x%08x\n",r);if(r)return 1;captured=true;CFRunLoopRunInMode(kCFRunLoopDefaultMode,1,false);
        r=(*device)->USBDeviceOpen(device);printf("DEVICE_REOPEN result=0x%08x\n",r);if(r)return 1;}
    if(!claim_button()){fputs("BUTTON_INTERFACE_UNAVAILABLE; returning device if captured.\n",stderr);return 1;}
    CFRunLoopSourceRef source=NULL;r=(*interface)->CreateInterfaceAsyncEventSource(interface,&source);if(r||!source){printf("ASYNC_SOURCE result=0x%08x\n",r);return 1;}
    CFRunLoopAddSource(CFRunLoopGetCurrent(),source,kCFRunLoopDefaultMode);
    if(!submit()){CFRunLoopRemoveSource(CFRunLoopGetCurrent(),source,kCFRunLoopDefaultMode);CFRelease(source);return 1;}
    if(share_video){r=(*device)->USBDeviceReEnumerate(device,kUSBReEnumerateReleaseDeviceMask);printf("SHARE_VIDEO_RETURN_TO_MACOS result=0x%08x\n",r);if(!r)captured=false;}
    puts("BUTTON_READY: press and release the physical button; observation lasts at most 60 seconds.");
    double deadline=CFAbsoluteTimeGetCurrent()+60;while(!stopped&&CFAbsoluteTimeGetCurrent()<deadline)CFRunLoopRunInMode(kCFRunLoopDefaultMode,.1,false);
    stopped=1;(*interface)->AbortPipe(interface,pipe_ref);CFRunLoopRunInMode(kCFRunLoopDefaultMode,.1,false);CFRunLoopRemoveSource(CFRunLoopGetCurrent(),source,kCFRunLoopDefaultMode);CFRelease(source);
    printf("BUTTON_DONE packets=%u\n",packets);cleanup();return packets?0:1;
}
