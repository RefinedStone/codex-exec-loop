# @refinedstone/akra

`@refinedstone/akra`는 `codex-exec-loop-native`를 npm으로 배포하는 패키지입니다.
설치 방식은 Codex CLI와 비슷하게 메타 패키지 + 플랫폼별 네이티브 optional dependency 조합을 사용합니다.

## 설치

```bash
npm install -g @refinedstone/akra
```

## 실행

```bash
cd /path/to/your/workspace
akra
```

## 업데이트

```bash
npm update -g @refinedstone/akra
```

## 제거

```bash
npm uninstall -g @refinedstone/akra
```

## 지원 플랫폼

- Linux `x64`
- macOS Apple Silicon `arm64`
- Windows `x64`

## 사전 조건

- `codex` CLI가 `PATH`에 있어야 합니다.
- `codex login`이 완료되어 있어야 합니다.
