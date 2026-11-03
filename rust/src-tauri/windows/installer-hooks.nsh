; Puts the DLLs promptflow.exe loads at startup next to it, so it starts on a fresh Windows too:
; the Visual C++ runtime (whisper.cpp and ONNX Runtime are C++), DirectML (ONNX Runtime links it and
; needs a newer one than older Windows 10 has) and the Vulkan loader (missing without a GPU driver;
; whisper.cpp then falls back to the CPU). rust/scripts/windows-runtime-dlls.sh copies them next to
; the built exe; paths are relative to target\release\nsis\x64\installer.nsi.

!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  File "..\..\msvcp140.dll"
  File "..\..\msvcp140_1.dll"
  File "..\..\vcruntime140.dll"
  File "..\..\vcruntime140_1.dll"
  File "..\..\DirectML.dll"
  File "..\..\vulkan-1.dll"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\msvcp140.dll"
  Delete "$INSTDIR\msvcp140_1.dll"
  Delete "$INSTDIR\vcruntime140.dll"
  Delete "$INSTDIR\vcruntime140_1.dll"
  Delete "$INSTDIR\DirectML.dll"
  Delete "$INSTDIR\vulkan-1.dll"
  RMDir "$INSTDIR"
!macroend
