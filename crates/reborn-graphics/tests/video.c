/* The production shader and video upload on a real software EGL/GLES context.
 * This checks host rendering, not DRM scanout or the Y2's GPU. */
#include "../native/graphics.c"
#include <assert.h>

static void check_pixel(int x, int y, int r, int green, int b) {
  uint8_t p[4];
  glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p);
  assert(abs((int)p[0] - r) < 8);
  assert(abs((int)p[1] - green) < 8);
  assert(abs((int)p[2] - b) < 8);
}

static void draw_video(RbGraphics *g) {
  rb_graphics_begin(g);
  rb_graphics_quad(g, 0, 0, 480, 360, 0xFFFFFFFF, 10, 0, 0, 0, 0);
  rb_graphics_submit(g);
}

int main(void) {
  PFNEGLGETPLATFORMDISPLAYEXTPROC get =
      (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
  assert(get);
  EGLDisplay display = get(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY, NULL);
  assert(display != EGL_NO_DISPLAY && eglInitialize(display, NULL, NULL));
  assert(eglBindAPI(EGL_OPENGL_ES_API));
  EGLint attributes[] = {EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
      EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_RED_SIZE, 8,
      EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_NONE};
  EGLConfig config;
  EGLint count;
  assert(eglChooseConfig(display, attributes, &config, 1, &count) && count);
  EGLint context_args[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
  EGLContext context = eglCreateContext(display, config, EGL_NO_CONTEXT, context_args);
  EGLint surface_args[] = {EGL_WIDTH, 480, EGL_HEIGHT, 360, EGL_NONE};
  EGLSurface surface = eglCreatePbufferSurface(display, config, surface_args);
  assert(context != EGL_NO_CONTEXT && surface != EGL_NO_SURFACE);
  assert(eglMakeCurrent(display, surface, surface, context));
  uint8_t page[1024] = {0}, mask[1] = {255};
  const uint8_t *pages[] = {page, page, page};
  int heights[] = {1, 1, 1};
  RbGraphics g = {.width = 480, .height = 360};
  assert(gl_setup(&g, pages, heights, mask, 1, 1) == 0);
  uint8_t picture[4 * 4 * 4];
  for (int y = 0; y < 4; y++) for (int x = 0; x < 4; x++) {
    uint8_t *p = picture + (y * 4 + x) * 4;
    p[0] = y < 2 && x < 2 ? 255 : 0;
    p[1] = y < 2 && x >= 2 ? 255 : 0;
    p[2] = y >= 2 ? 255 : 0;
    p[3] = 255;
  }
  assert(rb_graphics_video(&g, picture, 4, 4) == 0);
  GLuint texture_id = g.tex[10];
  draw_video(&g);
  check_pixel(60, 300, 255, 0, 0);
  check_pixel(420, 300, 0, 255, 0);
  check_pixel(60, 60, 0, 0, 255);
  for (int i = 0; i < 16; i++) { picture[i * 4] = 255; picture[i * 4 + 2] = 0; }
  assert(rb_graphics_video(&g, picture, 4, 4) == 0 && g.tex[10] == texture_id);
  draw_video(&g);
  check_pixel(60, 60, 255, 0, 0);
  assert(rb_graphics_video(&g, picture, 2, 2) == 0 && g.tex[10] == texture_id);
  assert(rb_graphics_video(&g, picture, 0, 4) == -EINVAL);
  assert(glGetError() == GL_NO_ERROR);
  glDeleteTextures(RB_TEXTURES, g.tex);
  glDeleteBuffers(1, &g.vbo);
  glDeleteProgram(g.program);
  eglMakeCurrent(display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
  eglDestroySurface(display, surface);
  eglDestroyContext(display, context);
  eglTerminate(display);
  return 0;
}
