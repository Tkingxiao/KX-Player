; 用户数据落在 <安装目录>\data（见 src-tauri/src/paths.rs）。
; perMachine 的默认安装位置是 Program Files，那里 Users 组只有读和执行权限 ——
; 不放写权限的话应用连数据目录都建不出来，曲库、设置、封面全部存不下。

!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$INSTDIR\data"
  ; S-1-5-32-545 是「Users」组的 SID：写 SID 而不是写名字，免得在中文/其他语言的系统上解析不到组名。
  ; (OI)(CI)M = 文件与子目录继承「修改」权限。只放开 data 这一个子目录：
  ; exe 与 libmpv-2.dll 所在的安装根目录保持只读给普通用户，否则任何程序都能换掉那个 DLL、
  ; 由下一个启动应用的用户加载执行（DLL 劫持）。
  nsExec::ExecToLog 'icacls "$INSTDIR\data" /grant "*S-1-5-32-545:(OI)(CI)M"'
!macroend

; 卸载时刻意不动 data\：Tauri 的卸载器只删自己铺过的文件，最后一步是不带 /r 的
; RMDir "$INSTDIR"，目录非空就留着 —— 所以曲库和设置能熬过卸载，重装即恢复。
; 想彻底清掉就手动删 <安装目录>\data。
