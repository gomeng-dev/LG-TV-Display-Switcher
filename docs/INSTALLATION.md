# LG TV Display Switcher 설치 가이드

이 문서는 `LG-TV-Display-Switcher` Windows 앱과 Stream Deck 플러그인을 설치하고 처음 설정하는 방법을 설명합니다.

English version: [INSTALLATION.en.md](INSTALLATION.en.md)

## 요구 사항

- Windows 10 이상
- LG webOS TV와 PC가 같은 로컬 네트워크에 연결되어 있어야 합니다.
- TV 전원 켜기 기능을 사용하려면 TV의 MAC 주소와 Wake-on-LAN 설정이 필요합니다.
- Stream Deck 플러그인을 사용하려면 Elgato Stream Deck 앱 7.1 이상을 권장합니다.

## Windows 앱 설치

1. GitHub 최신 릴리스 페이지를 엽니다.
   - https://github.com/gomeng-dev/LG-TV-Display-Switcher/releases/latest
2. `LG-TV-Display-Switcher-Setup.exe`를 내려받습니다.
3. 설치 파일을 실행합니다.
4. 설치 중 DisplayConfig PowerShell 모듈이 없으면 설치 과정에서 종속성 설치를 진행합니다.
5. 설치가 끝나면 `LG-TV-Display-Switcher`를 실행합니다.

Windows SmartScreen 또는 브라우저에서 경고가 나올 수 있습니다. 개인 배포 앱은 코드 서명 평판이 충분히 쌓이기 전까지 경고가 표시될 수 있습니다.

## 첫 실행 온보딩

1. 앱을 처음 실행하면 로컬 네트워크에서 LG TV를 검색합니다.
2. 검색된 TV를 선택하고 TV 화면에 표시되는 인증 요청을 승인합니다.
3. TV가 PC에 HDMI로 연결되어 있다면 TV 모드 전환을 적용합니다.
4. 오디오 장치 목록에서 TV 오디오 장치를 확인합니다.
   - 예: `LG TV SSCR2 (NVIDIA High Definition Audio)`
5. 온보딩이 끝나면 앱은 트레이 영역에서 실행됩니다.

## 트레이 메뉴

트레이 아이콘을 우클릭하면 다음 기능을 사용할 수 있습니다.

- `Apply TV Mode`: TV가 켜져 있을 때 TV 디스플레이 모드로 전환합니다.
- `Apply PC Mode`: PC 디스플레이 모드로 되돌립니다.
- `TV Power`: TV 전원을 켜거나 끕니다.
- `Auto Switch Displays`: TV 전원 상태 변화에 따라 TV/PC 모드를 자동 전환합니다.
- `Run as Startup`: Windows 로그인 시 앱을 자동 실행합니다.

PC 모드로 돌아갈 때 오디오는 TV 모드 진입 직전에 사용하던 기본 출력 장치로 복구됩니다.

## Stream Deck 플러그인 설치

1. GitHub 최신 릴리스 페이지에서 `dev.gomeng.lg-tv-display-switcher.streamDeckPlugin` 파일을 내려받습니다.
2. 파일을 더블 클릭합니다.
3. Stream Deck 앱에서 설치를 승인합니다.
4. Stream Deck 액션 목록에서 `LG TV Display Switcher` 카테고리를 찾습니다.
5. 원하는 액션을 Stream Deck 버튼에 배치합니다.

사용 가능한 액션:

- `TV Power Toggle`: TV 전원을 토글합니다.
- `Apply TV Mode`: TV 모드로 전환합니다. TV가 켜져 있을 때만 동작합니다.
- `Apply PC Mode`: PC 모드로 전환합니다.
- `Toggle Auto Switch`: 자동 전환 기능을 켜거나 끕니다.

Stream Deck 플러그인은 Windows 앱을 직접 포함하지 않습니다. 설치된 `LG-TV-Display-Switcher.exe`를 companion 앱으로 호출합니다. Windows 앱이 설치되어 있지 않으면 버튼에 `Install app` 또는 `App missing`이 표시되고 최신 릴리스 페이지가 열립니다.

## 설정 파일

앱 설정은 사용자 로컬 앱 데이터 경로에 저장됩니다. 일반적으로 다음 위치를 확인하면 됩니다.

```text
%LOCALAPPDATA%\LG-TV-Display-Switcher\
```

주요 설정:

- `TvHost`: LG TV IP 또는 호스트 이름
- `TvMac`: Wake-on-LAN에 사용할 TV MAC 주소
- `AutoSwitchDisplays`: TV 전원 상태에 따른 자동 디스플레이 전환 여부
- `AutoSwitchAudio`: TV/PC 모드 전환 시 기본 오디오 출력 전환 여부

## 문제 해결

### TV가 검색되지 않을 때

- PC와 TV가 같은 네트워크에 있는지 확인합니다.
- TV의 IP 주소를 고정하거나 공유기 DHCP 예약을 설정합니다.
- 앱 설정의 `TvHost`에 TV IP 주소를 직접 입력합니다.
- Windows 방화벽이 로컬 네트워크 통신을 막고 있지 않은지 확인합니다.

### TV 전원 켜기가 동작하지 않을 때

- TV 설정에서 Wake-on-LAN, Quick Start, Mobile TV On 같은 네트워크 대기 기능을 켭니다.
- `TvMac` 값이 정확한지 확인합니다.
- 유선 LAN이 Wi-Fi보다 Wake-on-LAN 성공률이 높습니다.

### TV 모드가 적용되지 않을 때

- TV가 실제로 켜져 있는지 확인합니다.
- TV가 Windows에서 디스플레이로 감지되는지 확인합니다.
- HDMI 케이블 또는 GPU 출력 포트를 확인합니다.
- `Apply TV Mode`는 TV가 꺼진 상태에서는 실패하도록 설계되어 있습니다.

### 오디오가 TV로 바뀌지 않을 때

- Windows 소리 설정에서 TV 오디오 장치가 표시되는지 확인합니다.
- TV 모드 진입 후 TV 오디오 장치가 늦게 생성될 수 있으므로 몇 초 뒤 다시 시도합니다.
- 온보딩에서 표시된 TV 오디오 이름이 실제 Windows 장치 이름과 일치하는지 확인합니다.

### Stream Deck 버튼이 `App missing`으로 표시될 때

- Windows 앱이 먼저 설치되어 있어야 합니다.
- 설치 후 Stream Deck 앱을 재시작합니다.
- 플러그인은 다음 순서로 앱을 찾습니다.
  - `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\LG-TV-Display-Switcher`의 `InstallLocation`
  - `%LOCALAPPDATA%\LG-TV-Display-Switcher\LG-TV-Display-Switcher.exe`

## 제거

1. Windows 설정의 앱 목록에서 `LG-TV-Display-Switcher`를 제거합니다.
2. Stream Deck 앱에서 `LG TV Display Switcher` 플러그인을 제거합니다.
3. 필요하면 `%LOCALAPPDATA%\LG-TV-Display-Switcher\` 설정 폴더를 삭제합니다.
