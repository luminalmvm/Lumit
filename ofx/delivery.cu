// Convert Lumit's display texture into Resolve's output, entirely on the GPU.
#include <cuda_runtime.h>
extern "C" __global__ void deliver(cudaTextureObject_t source, float4* output,
    unsigned width, unsigned height, int4 bounds, int4 rod, int stride, const uchar4* pixels) {
    int x=bounds.x+blockIdx.x*blockDim.x+threadIdx.x;
    int y=bounds.y+blockIdx.y*blockDim.y+threadIdx.y;
    if(x>=bounds.z || y>=bounds.w) return;
    float4 pixel=make_float4(0,0,0,0);
    if(source || pixels) {
        int sx=max(0,min(int(width)-1,int(((2LL*(x-rod.x)+1)*width)/(2LL*(rod.z-rod.x)))));
        int sy=max(0,min(int(height)-1,int(((2LL*(rod.w-y)-1)*height)/(2LL*(rod.w-rod.y)))));
        uchar4 p=source ? tex2D<uchar4>(source,float(sx)+0.5f,float(sy)+0.5f) : pixels[size_t(sy)*width+sx];
        pixel=make_float4((source?p.z:p.x)/255.0f,p.y/255.0f,(source?p.x:p.z)/255.0f,p.w/255.0f);
    }
    auto row=reinterpret_cast<float4*>(reinterpret_cast<char*>(output)+ptrdiff_t(y-bounds.y)*stride);
    row[x-bounds.x]=pixel;
}
