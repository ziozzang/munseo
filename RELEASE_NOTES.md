HWP2PDF 0.1.1

- **연결 프로그램 등록**
  - macOS: Finder "다음으로 열기" 목록에 HWP2PDF가 나오고, Dock 아이콘에 파일을 놓아도 변환합니다.
  - Windows: `install.ps1`로 설치하면 "연결 프로그램" 목록과 우클릭 메뉴 "HWP2PDF로 변환"이 등록됩니다. 기본 앱은 바꾸지 않습니다.
- **수정**: 진행 목록에서 한글 파일 이름이 네모로 보이던 문제. macOS의 자모 분리형(NFD) 이름을 음절로 합쳐 표시합니다.
- **수정**: Windows 실행 파일에 아이콘이 들어갑니다.

설치 (경고 없이):
- macOS: `curl -fsSL https://raw.githubusercontent.com/ziozzang/munseo/main/install.sh | sh`
- Windows: `irm https://raw.githubusercontent.com/ziozzang/munseo/main/install.ps1 | iex`

0.1.0을 쓰고 있다면 앱이 알아서 업데이트를 알려 줍니다.
