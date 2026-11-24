#include <cuda_runtime.h>
#include <omp.h>
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
struct V3 { float v[3]; };
static void check(cudaError_t error) {
  if (error != cudaSuccess) { std::fprintf(stderr, "%s\n", cudaGetErrorString(error)); std::exit(1); }
}
__host__ __device__ void row(const V3* pixels, V3* out, int width, int radius, int y) {
  double sum[3] = {};
  for (int x=0; x<=min(radius,width-1); ++x)
    for (int c=0;c<3;++c) sum[c] += (double)pixels[y*width+x].v[c];
  for (int x=0;x<width;++x) {
    const int count=min(x+radius+1,width)-max(x-radius,0);
    for (int c=0;c<3;++c) out[y*width+x].v[c]=(float)(sum[c]/(double)count);
    for (int c=0;c<3;++c) {
      if (x>=radius) sum[c] -= (double)pixels[y*width+x-radius].v[c];
      if (x+radius+1<width) sum[c] += (double)pixels[y*width+x+radius+1].v[c];
    }
  }
}
__host__ __device__ void column(const V3* pixels, V3* out, int width, int height, int radius, int x) {
  double sum[3] = {};
  for (int y=0;y<=min(radius,height-1);++y)
    for (int c=0;c<3;++c) sum[c] += (double)pixels[y*width+x].v[c];
  for (int y=0;y<height;++y) {
    const int count=min(y+radius+1,height)-max(y-radius,0);
    for (int c=0;c<3;++c) out[y*width+x].v[c]=(float)(sum[c]/(double)count);
    for (int c=0;c<3;++c) {
      if (y>=radius) sum[c] -= (double)pixels[(y-radius)*width+x].v[c];
      if (y+radius+1<height) sum[c] += (double)pixels[(y+radius+1)*width+x].v[c];
    }
  }
}
__global__ void horizontal(const V3* input,V3* out,int w,int h,int batch,int radius) {
  const int i=blockIdx.x*blockDim.x+threadIdx.x;
  if(i<batch*h) { const int patch=i/h; row(input+patch*w*h,out+patch*w*h,w,radius,i%h); }
}
__global__ void vertical(const V3* input,V3* out,int w,int h,int batch,int radius) {
  const int i=blockIdx.x*blockDim.x+threadIdx.x;
  if(i<batch*w) { const int patch=i/w; column(input+patch*w*h,out+patch*w*h,w,h,radius,i%w); }
}
static void gpu(const V3* input,V3* tmp,V3* out,int w,int h,int batch,int radius) {
  horizontal<<<(batch*h+127)/128,128>>>(input,tmp,w,h,batch,radius);
  vertical<<<(batch*w+127)/128,128>>>(tmp,out,w,h,batch,radius);
}
int main() {
  omp_set_num_threads(4);
  check(cudaSetDevice(0));
  for(int batch: {1,64,2048}) {
    const int w=24,h=40,radius=4;
    const size_t count=(size_t)batch*w*h, bytes=count*sizeof(V3);
    std::vector<V3> source(count),tmp(count),expected(count),actual(count);
    unsigned state=12345;
    for(auto& pixel:source) for(auto& c:pixel.v) {state=1664525*state+1013904223; c=(float)(state%65536)/65535.0f-0.5f;}
    V3 *input,*scratch,*output;
    check(cudaMalloc(&input,bytes)); check(cudaMalloc(&scratch,bytes)); check(cudaMalloc(&output,bytes));
    check(cudaMemcpy(input,source.data(),bytes,cudaMemcpyHostToDevice));
    gpu(input,scratch,output,w,h,batch,radius); check(cudaDeviceSynchronize());
    const int repeats=batch==2048?8:100;
    auto start=std::chrono::steady_clock::now();
    for(int repeat=0;repeat<repeats;++repeat) {
      #pragma omp parallel for if(batch>1)
      for(int b=0;b<batch;++b) {
        const size_t offset=(size_t)b*w*h;
        for(int y=0;y<h;++y) row(source.data()+offset,tmp.data()+offset,w,radius,y);
        for(int x=0;x<w;++x) column(tmp.data()+offset,expected.data()+offset,w,h,radius,x);
      }
    }
    const double cpu_ms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-start).count()/repeats;
    cudaEvent_t a,b; check(cudaEventCreate(&a)); check(cudaEventCreate(&b));
    check(cudaEventRecord(a));
    for(int repeat=0;repeat<repeats;++repeat) gpu(input,scratch,output,w,h,batch,radius);
    check(cudaEventRecord(b)); check(cudaEventSynchronize(b));
    float gpu_total; check(cudaEventElapsedTime(&gpu_total,a,b));
    check(cudaMemcpy(actual.data(),output,bytes,cudaMemcpyDeviceToHost));
    if(std::memcmp(actual.data(),expected.data(),bytes)!=0) {
      size_t mismatches=0;
      for(size_t i=0;i<count;++i) mismatches += std::memcmp(&actual[i],&expected[i],sizeof(V3))!=0;
      std::fprintf(stderr,"bitwise mismatch: %zu of %zu pixels\n",mismatches,count); return 2;
    }
    start=std::chrono::steady_clock::now();
    for(int repeat=0;repeat<repeats;++repeat) {
      check(cudaMemcpy(input,source.data(),bytes,cudaMemcpyHostToDevice));
      gpu(input,scratch,output,w,h,batch,radius);
      check(cudaMemcpy(actual.data(),output,bytes,cudaMemcpyDeviceToHost));
    }
    const double roundtrip_ms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-start).count()/repeats;
    std::printf("batch=%d CPU_4threads_ms=%.6f GPU_resident_ms=%.6f GPU_roundtrip_ms=%.6f identical=yes\n",batch,cpu_ms,gpu_total/repeats,roundtrip_ms);
    check(cudaFree(input)); check(cudaFree(scratch)); check(cudaFree(output));
    check(cudaEventDestroy(a)); check(cudaEventDestroy(b));
  }
}
