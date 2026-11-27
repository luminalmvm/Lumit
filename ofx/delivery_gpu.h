#pragma once
#include <windows.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>
#include <cuda.h>
#include <cudaD3D11.h>
#include <stdexcept>
#include <string>
#include <vector>

// Only pixel delivery. CUDA is loaded lazily; CPU hosts need no NVIDIA DLL.
class CudaDelivery {
#define CUDA_CALLS(X) \
 X(cuInit) X(cuCtxGetCurrent) X(cuCtxGetDevice) X(cuCtxPushCurrent_v2) X(cuCtxPopCurrent_v2) \
 X(cuModuleLoadData) X(cuModuleUnload) X(cuModuleGetFunction) X(cuD3D11GetDevice) \
 X(cuGraphicsD3D11RegisterResource) X(cuGraphicsUnregisterResource) X(cuGraphicsMapResources) \
 X(cuGraphicsUnmapResources) X(cuGraphicsSubResourceGetMappedArray) X(cuTexObjectCreate) \
 X(cuTexObjectDestroy) X(cuLaunchKernel) X(cuStreamSynchronize) \
 X(cuMemAlloc_v2) X(cuMemFree_v2) X(cuMemcpyHtoD_v2)
#define DECLARE(name) decltype(&name) name##_{};
    CUDA_CALLS(DECLARE)
#undef DECLARE
    HMODULE driver{};
    bool ready{};
    CUcontext owner{};
    CUmodule module{};
    CUfunction kernel{};
    CUgraphicsResource resource{};
    uint64_t handle{};
    Microsoft::WRL::ComPtr<ID3D11Device> device;
    Microsoft::WRL::ComPtr<ID3D11Texture2D> texture;
    static void check(CUresult result) {
        if(result!=CUDA_SUCCESS) throw std::runtime_error("CUDA frame delivery failed ("+std::to_string(int(result))+").");
    }
    void initialize() {
        CUcontext current{}; check(cuCtxGetCurrent_(&current));
        if(!current) throw std::runtime_error("Resolve supplied no CUDA context.");
        if(owner==current && kernel) return;
        reset(); owner=current;
        CUdevice target{}; check(cuCtxGetDevice_(&target));
        Microsoft::WRL::ComPtr<IDXGIFactory1> factory;
        if(FAILED(CreateDXGIFactory1(IID_PPV_ARGS(&factory)))) throw std::runtime_error("Cannot enumerate delivery adapters.");
        for(UINT n=0; !device; ++n) {
            Microsoft::WRL::ComPtr<IDXGIAdapter1> adapter;
            if(factory->EnumAdapters1(n,&adapter)==DXGI_ERROR_NOT_FOUND) break;
            CUdevice found{};
            if(cuD3D11GetDevice_(&found,adapter.Get())!=CUDA_SUCCESS || found!=target) continue;
            D3D_FEATURE_LEVEL level{};
            if(FAILED(D3D11CreateDevice(adapter.Get(),D3D_DRIVER_TYPE_UNKNOWN,nullptr,0,nullptr,0,
                D3D11_SDK_VERSION,&device,&level,nullptr))) throw std::runtime_error("Cannot open the Resolve delivery adapter.");
        }
        if(!device) throw std::runtime_error("Lumit and Resolve must use the same GPU.");
        HMODULE binary{};
        GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            reinterpret_cast<LPCWSTR>(&supported),&binary);
        HRSRC entry=FindResourceW(binary,MAKEINTRESOURCEW(102),MAKEINTRESOURCEW(10));
        if(!entry) throw std::runtime_error("Missing GPU delivery kernel.");
        const auto data=static_cast<const char*>(LockResource(LoadResource(binary,entry)));
        std::string ptx(data,SizeofResource(binary,entry));
        check(cuModuleLoadData_(&module,ptx.c_str()));
        check(cuModuleGetFunction_(&kernel,module,"deliver"));
    }
public:
    bool available() {
        if(driver) return ready;
        driver=LoadLibraryExW(L"nvcuda.dll",nullptr,LOAD_LIBRARY_SEARCH_SYSTEM32);
        if(!driver) return false;
#define LOAD(name) name##_=reinterpret_cast<decltype(name##_)>(GetProcAddress(driver,#name)); if(!name##_) return false;
        CUDA_CALLS(LOAD)
#undef LOAD
        ready=cuInit_(0)==CUDA_SUCCESS;
        return ready;
    }
    static bool supported() { CudaDelivery candidate; return candidate.available(); }
    ~CudaDelivery() { reset(); if(driver) FreeLibrary(driver); }
    void reset() {
        if(owner && cuCtxPushCurrent_v2_ && cuCtxPushCurrent_v2_(owner)==CUDA_SUCCESS) {
            if(resource) cuGraphicsUnregisterResource_(resource);
            if(module) cuModuleUnload_(module);
            CUcontext previous{}; cuCtxPopCurrent_v2_(&previous);
        }
        owner=nullptr; resource=nullptr; module=nullptr; kernel=nullptr; handle=0;
        texture.Reset(); device.Reset();
    }
    void copy(uint64_t shared, unsigned width,unsigned height,void* output,
              int* bounds,int* rod,int stride,void* queue,const unsigned char* pixels=nullptr) {
        if(!available()) throw std::runtime_error("CUDA frame delivery is unavailable.");
        initialize();
        auto stream=static_cast<CUstream>(queue);
        if(shared && handle!=shared) {
            if(resource) { check(cuGraphicsUnregisterResource_(resource)); resource=nullptr; }
            texture.Reset(); handle=0;
            if(FAILED(device->OpenSharedResource(reinterpret_cast<HANDLE>(uintptr_t(shared)),IID_PPV_ARGS(&texture))))
                throw std::runtime_error("Cannot open Lumit's GPU surface. Use the same GPU in both apps.");
            check(cuGraphicsD3D11RegisterResource_(&resource,texture.Get(),CU_GRAPHICS_REGISTER_FLAGS_NONE));
            handle=shared;
        }
        CUtexObject sampler{}; CUdeviceptr upload{}; bool mapped=false;
        try {
            if(pixels) {
                const size_t bytes=size_t(width)*height*4;
                check(cuMemAlloc_v2_(&upload,bytes)); check(cuMemcpyHtoD_v2_(upload,pixels,bytes));
            }
            if(shared) {
                check(cuGraphicsMapResources_(1,&resource,stream)); mapped=true;
                CUarray array{}; check(cuGraphicsSubResourceGetMappedArray_(&array,resource,0,0));
                CUDA_RESOURCE_DESC source{}; source.resType=CU_RESOURCE_TYPE_ARRAY; source.res.array.hArray=array;
                CUDA_TEXTURE_DESC sampling{}; sampling.flags=CU_TRSF_READ_AS_INTEGER;
                sampling.addressMode[0]=sampling.addressMode[1]=CU_TR_ADDRESS_MODE_CLAMP;
                check(cuTexObjectCreate_(&sampler,&source,&sampling,nullptr));
            }
            void* args[]{&sampler,&output,&width,&height,bounds,rod,&stride,&upload};
            check(cuLaunchKernel_(kernel,(bounds[2]-bounds[0]+15)/16,(bounds[3]-bounds[1]+15)/16,1,
                16,16,1,0,stream,args,nullptr));
            if(mapped) { check(cuGraphicsUnmapResources_(1,&resource,stream)); mapped=false; }
            // The next request may overwrite Lumit's one delivery surface.
            check(cuStreamSynchronize_(stream));
            if(upload) { check(cuMemFree_v2_(upload)); upload=0; }
            if(sampler) check(cuTexObjectDestroy_(sampler));
        } catch(...) {
            if(mapped) cuGraphicsUnmapResources_(1,&resource,stream);
            cuStreamSynchronize_(stream);
            if(upload) cuMemFree_v2_(upload);
            if(sampler) cuTexObjectDestroy_(sampler);
            throw;
        }
    }
#undef CUDA_CALLS
};
