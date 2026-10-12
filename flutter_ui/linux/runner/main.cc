#include "gpu_offload.h"
#include "my_application.h"

int main(int argc, char** argv) {
  // Before anything of GTK's. On a laptop with an NVIDIA card beside another
  // one this asks for Flutter to be drawn on the NVIDIA card, the card the
  // renderer is on, and the driver only listens while the context is being
  // made. Every other machine is left alone. See gpu_offload.h.
  lumit_request_gpu_offload();

  g_autoptr(MyApplication) app = my_application_new();
  return g_application_run(G_APPLICATION(app), argc, argv);
}
