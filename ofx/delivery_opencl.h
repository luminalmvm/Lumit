#pragma once
#include <windows.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>
#define CL_TARGET_OPENCL_VERSION 300
#include <CL/cl.h>
#include <CL/cl_ext.h>
#include <stdexcept>
#include <string>
#include <vector>
#include <algorithm>

// Uses the host's context and queue. No NVIDIA API or separate OpenCL context.
class OpenCLDelivery {
#define CL_CALLS(X) \
 X(clGetCommandQueueInfo) X(clGetDeviceInfo) X(clGetExtensionFunctionAddressForPlatform) \
 X(clCreateProgramWithSource) X(clBuildProgram) X(clGetProgramBuildInfo) X(clCreateKernel) \
 X(clSetKernelArg) X(clEnqueueNDRangeKernel) X(clWaitForEvents) X(clReleaseEvent) \
 X(clReleaseKernel) X(clReleaseProgram) X(clReleaseMemObject) X(clCreateImage) X(clEnqueueWriteImage)
#define DECLARE(name) decltype(&name) name##_{};
    CL_CALLS(DECLARE)
#undef DECLARE
    decltype(&clCreateImageWithProperties) createImage{};
    clEnqueueAcquireExternalMemObjectsKHR_fn acquire{};
    clEnqueueReleaseExternalMemObjectsKHR_fn release{};
    HMODULE library{};
    bool ready{};
    cl_context context{};
    cl_device_id device{};
    cl_program program{};
    cl_kernel kernel{};
    cl_mem image{};
    cl_mem_properties handleType{};
    uint64_t handle{};
    unsigned imageWidth{},imageHeight{};
    Microsoft::WRL::ComPtr<ID3D11Device> surfaceDevice;
    Microsoft::WRL::ComPtr<ID3D11Texture2D> surface;
    void holdSurface(uint64_t shared) {
        surface.Reset();
        if(surfaceDevice && SUCCEEDED(surfaceDevice->OpenSharedResource(
            reinterpret_cast<HANDLE>(uintptr_t(shared)),IID_PPV_ARGS(&surface)))) return;
        surfaceDevice.Reset();
        Microsoft::WRL::ComPtr<IDXGIFactory1> factory;
        if(FAILED(CreateDXGIFactory1(IID_PPV_ARGS(&factory)))) throw std::runtime_error("Cannot enumerate delivery adapters.");
        for(UINT n=0;;++n) {
            Microsoft::WRL::ComPtr<IDXGIAdapter1> adapter;
            if(factory->EnumAdapters1(n,&adapter)==DXGI_ERROR_NOT_FOUND) break;
            Microsoft::WRL::ComPtr<ID3D11Device> candidate;
            D3D_FEATURE_LEVEL level{};
            if(SUCCEEDED(D3D11CreateDevice(adapter.Get(),D3D_DRIVER_TYPE_UNKNOWN,nullptr,0,nullptr,0,
                D3D11_SDK_VERSION,&candidate,&level,nullptr)) && SUCCEEDED(candidate->OpenSharedResource(
                reinterpret_cast<HANDLE>(uintptr_t(shared)),IID_PPV_ARGS(&surface)))) {
                surfaceDevice=candidate;return;
            }
        }
        throw std::runtime_error("Cannot open Lumit's GPU surface. Use the same GPU in both apps.");
    }
    static void check(cl_int status) {
        if(status!=CL_SUCCESS) throw std::runtime_error("OpenCL frame delivery failed ("+std::to_string(status)+").");
    }
    void initialize(cl_command_queue queue) {
        if(!queue || !available()) throw std::runtime_error("Resolve supplied no OpenCL queue.");
        cl_context owner{};cl_device_id target{};
        check(clGetCommandQueueInfo_(queue,CL_QUEUE_CONTEXT,sizeof(owner),&owner,nullptr));
        check(clGetCommandQueueInfo_(queue,CL_QUEUE_DEVICE,sizeof(target),&target,nullptr));
        if(context==owner && device==target && kernel) return;
        reset();context=owner;device=target;
        cl_platform_id platform{};
        check(clGetDeviceInfo_(device,CL_DEVICE_PLATFORM,sizeof(platform),&platform,nullptr));
        acquire=reinterpret_cast<clEnqueueAcquireExternalMemObjectsKHR_fn>(clGetExtensionFunctionAddressForPlatform_(platform,"clEnqueueAcquireExternalMemObjectsKHR"));
        release=reinterpret_cast<clEnqueueReleaseExternalMemObjectsKHR_fn>(clGetExtensionFunctionAddressForPlatform_(platform,"clEnqueueReleaseExternalMemObjectsKHR"));
        size_t bytes{};
        if(createImage && acquire && release && clGetDeviceInfo_(device,CL_DEVICE_EXTERNAL_MEMORY_IMPORT_HANDLE_TYPES_KHR,0,nullptr,&bytes)==CL_SUCCESS && bytes && bytes%sizeof(cl_external_memory_handle_type_khr)==0) {
            std::vector<cl_external_memory_handle_type_khr> types(bytes/sizeof(cl_external_memory_handle_type_khr));
            check(clGetDeviceInfo_(device,CL_DEVICE_EXTERNAL_MEMORY_IMPORT_HANDLE_TYPES_KHR,bytes,types.data(),nullptr));
            for(auto type:{CL_EXTERNAL_MEMORY_HANDLE_D3D11_TEXTURE_KMT_KHR,CL_EXTERNAL_MEMORY_HANDLE_OPAQUE_WIN32_KMT_KHR}) {
                if(std::find(types.begin(),types.end(),type)!=types.end()){handleType=type;break;}
            }
        }
        const char* source=R"cl(
__kernel void deliver(read_only image2d_t source,__global float4* output,
    uint width,uint height,int4 bounds,int4 rod,int stride) {
    int x=bounds.x+get_global_id(0),y=bounds.y+get_global_id(1);
    int sx=clamp((int)(((2L*(x-rod.x)+1)*width)/(2L*(rod.z-rod.x))),0,(int)width-1);
    int sy=clamp((int)(((2L*(rod.w-y)-1)*height)/(2L*(rod.w-rod.y))),0,(int)height-1);
    const sampler_t sample=CLK_NORMALIZED_COORDS_FALSE|CLK_ADDRESS_CLAMP_TO_EDGE|CLK_FILTER_NEAREST;
    float4 pixel=read_imagef(source,sample,(int2)(sx,sy));
    __global float4* row=(__global float4*)((__global char*)output+(long)(y-bounds.y)*stride);
    row[x-bounds.x]=pixel;
})cl";
        cl_int status{};program=clCreateProgramWithSource_(context,1,&source,nullptr,&status);check(status);
        status=clBuildProgram_(program,1,&device,"-cl-std=CL1.2",nullptr,nullptr);
        if(status!=CL_SUCCESS){size_t size{};clGetProgramBuildInfo_(program,device,CL_PROGRAM_BUILD_LOG,0,nullptr,&size);std::string log(size,0);clGetProgramBuildInfo_(program,device,CL_PROGRAM_BUILD_LOG,size,log.data(),nullptr);throw std::runtime_error("OpenCL delivery kernel: "+log);}
        kernel=clCreateKernel_(program,"deliver",&status);check(status);
    }
public:
    bool available() {
        if(library) return ready;
        library=LoadLibraryExW(L"OpenCL.dll",nullptr,LOAD_LIBRARY_SEARCH_SYSTEM32);
        if(!library) return false;
#define LOAD(name) name##_=reinterpret_cast<decltype(name##_)>(GetProcAddress(library,#name)); if(!name##_) return false;
        CL_CALLS(LOAD)
#undef LOAD
        createImage=reinterpret_cast<decltype(createImage)>(GetProcAddress(library,"clCreateImageWithProperties"));
        ready=true;return true;
    }
    static bool supported(){OpenCLDelivery candidate;return candidate.available();}
    ~OpenCLDelivery(){reset();if(library)FreeLibrary(library);}
    void reset(){
        if(image)clReleaseMemObject_(image);if(kernel)clReleaseKernel_(kernel);if(program)clReleaseProgram_(program);
        image=nullptr;kernel=nullptr;program=nullptr;context=nullptr;device=nullptr;handle=0;handleType=0;imageWidth=imageHeight=0;
        surface.Reset();surfaceDevice.Reset();
    }
    bool canShare(void* queue){initialize(static_cast<cl_command_queue>(queue));return handleType!=0;}
    void copy(uint64_t shared,unsigned width,unsigned height,const unsigned char* pixels,
              void* output,int* bounds,int* rod,int stride,void* commandQueue) {
        if(stride<=0)throw std::runtime_error("Resolve supplied an invalid OpenCL output stride.");
        auto queue=static_cast<cl_command_queue>(commandQueue);initialize(queue);
        if(!image || handle!=shared || width!=imageWidth || height!=imageHeight) {
            if(image){check(clReleaseMemObject_(image));image=nullptr;}
            cl_image_desc desc{};desc.image_type=CL_MEM_OBJECT_IMAGE2D;desc.image_width=width;desc.image_height=height;
            cl_image_format format{cl_channel_order(shared?CL_BGRA:CL_RGBA),CL_UNORM_INT8};cl_int status{};
            if(shared){
                if(!handleType)throw std::runtime_error("This OpenCL driver cannot share Lumit's GPU texture.");
                // KMT handles do not own the resource: hold an opened D3D view
                // until the OpenCL image is released, including between frames.
                holdSurface(shared);
                cl_mem_properties properties[]{handleType,cl_mem_properties(shared),0};
                image=createImage(context,properties,CL_MEM_READ_ONLY,&format,&desc,nullptr,&status);
            } else {surface.Reset();image=clCreateImage_(context,CL_MEM_READ_ONLY,&format,&desc,nullptr,&status);}
            check(status);handle=shared;imageWidth=width;imageHeight=height;
        }
        cl_event acquired{},drawn{},released{};
        try {
            if(shared)check(acquire(queue,1,&image,0,nullptr,&acquired));
            else {size_t origin[3]{},region[3]{width,height,1};check(clEnqueueWriteImage_(queue,image,CL_TRUE,origin,region,0,0,pixels,0,nullptr,nullptr));}
            size_t sizes[]{sizeof(image),sizeof(cl_mem),sizeof(width),sizeof(height),sizeof(int)*4,sizeof(int)*4,sizeof(stride)};
            const void* args[]{&image,&output,&width,&height,bounds,rod,&stride};
            for(cl_uint i=0;i<7;++i)check(clSetKernelArg_(kernel,i,sizes[i],args[i]));
            size_t extent[]{size_t(bounds[2]-bounds[0]),size_t(bounds[3]-bounds[1])};
            check(clEnqueueNDRangeKernel_(queue,kernel,2,nullptr,extent,nullptr,acquired?1:0,acquired?&acquired:nullptr,&drawn));
            if(shared)check(release(queue,1,&image,1,&drawn,&released));
            // Explicit dependencies also support an out-of-order host queue.
            check(clWaitForEvents_(1,released?&released:&drawn));
        } catch(...) {
            if(acquired && !released)release(queue,1,&image,drawn?1:0,drawn?&drawn:nullptr,&released);
            if(released)clWaitForEvents_(1,&released);
            if(acquired)clReleaseEvent_(acquired);if(drawn)clReleaseEvent_(drawn);if(released)clReleaseEvent_(released);
            throw;
        }
        if(acquired)clReleaseEvent_(acquired);if(drawn)clReleaseEvent_(drawn);if(released)clReleaseEvent_(released);
    }
#undef CL_CALLS
};
