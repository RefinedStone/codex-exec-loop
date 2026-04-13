# CKA 네트워크 최소 핵심 90분 강의안

## 강의 개요

- 강의명: `CKA 합격을 위한 쿠버네티스 네트워크 최소 핵심`
- 강의 길이: 90분
- 대상: Kubernetes 기초 리소스는 알지만 네트워크는 시험 직전 최소 범위만 빠르게 정리하고 싶은 수강생
- 강의 목표: 수강생이 시험에서 네트워크 문제를 만나면 `Pod -> Service -> DNS -> Ingress/NetworkPolicy` 순서로 줄여 가며 판단할 수 있게 한다.
- 전달 원칙: 구현 심화보다 시험 문제 해결 순서와 명령어 사용 기준을 남긴다.
- 슬라이드 설계안: [cka-network-90min-slide-plan.md](/Users/newin_mac/Documents/dev/akra/codex-exec-loop-worktrees/docs-native-platform-presentation-cka-network-90min/cka-network-90min-slide-plan.md)
- HTML 드래프트: [slides/cka-network-90min-draft/index.html](/Users/newin_mac/Documents/dev/akra/codex-exec-loop-worktrees/docs-native-platform-presentation-cka-network-90min/slides/cka-network-90min-draft/index.html)

## 이번 강의에서 가져가야 할 것

- Pod 간 통신과 Service 경유 통신의 차이를 설명할 수 있다.
- `ClusterIP`, `NodePort`, `LoadBalancer`, `headless Service`를 시험 관점에서 구분할 수 있다.
- `CoreDNS`, 서비스 이름 해석, namespace 포함 FQDN 개념을 이해한다.
- `Ingress`와 `NetworkPolicy`를 "어디를 열고 어디를 막는 리소스인가" 수준으로 해석할 수 있다.
- `kubectl get/describe/exec/logs`와 `nslookup`, `dig`, `curl`, `wget`으로 네트워크 장애를 1차 분류할 수 있다.

## 이번 강의에서 과감히 빼는 것

- OSI 7계층 상세
- 라우팅 프로토콜 심화
- kube-proxy 내부 구현 세부
- CNI 플러그인 구현 차이와 BGP/VXLAN 심화
- Ingress Controller 설치 절차 심화

## 90분 구성표

| 시간 | 섹션 | 핵심 메시지 | 진행 방식 |
| --- | --- | --- | --- |
| 0:00-0:05 | 오프닝 | 시험에서는 "네트워크 이론"보다 "통신 경로를 줄이는 순서"가 중요하다 | 설명 |
| 0:05-0:15 | 네트워크 기초 10분 압축 | IP, Port, TCP/UDP, CIDR, DNS만 알면 충분하다 | 설명 + 짧은 예시 |
| 0:15-0:27 | 쿠버네티스 네트워크 3원칙 | 모든 Pod는 IP를 갖고, Pod 간 통신이 가능해야 하며, 노드가 달라도 이어져야 한다 | 설명 + 그림 |
| 0:27-0:42 | Service 핵심 | Service는 Pod 앞에 고정 진입점과 로드밸런싱을 만든다 | 설명 + 데모 |
| 0:42-0:52 | DNS 핵심 | 시험에서는 IP보다 서비스 이름으로 통신하는 흐름을 읽어야 한다 | 설명 + 데모 |
| 0:52-1:02 | Ingress 최소 이해 | Ingress는 외부 요청을 어느 Service로 보낼지 정하는 규칙이다 | 설명 + YAML 읽기 |
| 1:02-1:15 | NetworkPolicy 최소 이해 | NetworkPolicy는 기본 차단이 아니라 "선택된 Pod에 허용 규칙을 붙이는 리소스"다 | 설명 + 데모 |
| 1:15-1:26 | 트러블슈팅 드릴 | Pod to Pod, Pod to Service, DNS, Policy 문제를 순서대로 줄인다 | 실전 문제 풀이 |
| 1:26-1:30 | 마무리 | 시험 직전 암기 포인트와 체크리스트를 남긴다 | 정리 |

## 섹션별 상세안

### 1. 오프닝 `5분`

- 학습 목표: 네트워크를 "공부할 범위"가 아니라 "시험 문제를 줄이는 순서"로 인식시킨다.
- 시작 질문: `curl`이 안 될 때 Pod 문제인지 Service 문제인지 DNS 문제인지 어떻게 나눌 것인가.
- 강사용 핵심 문장: CKA 네트워크는 "전부 이해"가 아니라 "어디서 끊기는지 줄이는 능력"이다.
- 슬라이드 포인트:
  - CKA에서 네트워크가 자주 엮이는 영역은 `Services & Networking 20%`와 `Troubleshooting 30%`
  - 오늘 강의의 목표는 "구현 원리 암기"가 아니라 "장애 위치 추정"

### 2. 네트워크 기초 10분 압축 `10분`

- 학습 목표: 쿠버네티스 네트워크를 이해하는 데 필요한 최소 용어만 맞춘다.
- 설명할 개념:
  - IP: 통신 대상 주소
  - Port: 한 IP 안의 서비스 출입구
  - TCP/UDP: 시험에서는 대부분 TCP 중심으로 이해하면 충분
  - CIDR: `10.96.0.0/12`, `192.168.0.0/24` 같은 대역 표기
  - DNS: 이름을 IP로 바꾸는 시스템
- 강사용 예시:
  - "브라우저에서 `example.com:443`에 접속한다"를 이름, 주소, 포트로 분해
  - "Service DNS 이름으로 붙지만 실제 연결 대상은 Pod"라는 문장으로 연결
- 여기서 멈출 것:
  - subnetting 계산 문제를 길게 끌지 않는다.
  - TCP 3-way handshake를 설명하지 않는다.

### 3. 쿠버네티스 네트워크 3원칙 `12분`

- 학습 목표: Pod 네트워크를 처음 보는 수강생도 전체 흐름을 한 장으로 이해하게 한다.
- 반드시 남길 세 문장:
  - 모든 Pod는 자신의 IP를 가진다.
  - 같은 노드든 다른 노드든 Pod 간 직접 통신이 가능해야 한다.
  - Pod는 죽고 다시 뜰 수 있으므로, 고정 진입점은 Pod가 아니라 Service가 맡는다.
- 설명할 내용:
  - Pod IP는 재시작 시 바뀔 수 있다.
  - 여러 컨테이너가 한 Pod 안에 있으면 네트워크 네임스페이스를 공유한다.
  - CNI는 "Pod에 네트워크를 붙여 주는 규칙/플러그인" 정도까지만 설명한다.
- 판서 또는 슬라이드 그림:
  - Node A의 Pod 1, Node B의 Pod 2, 앞단의 Service, 옆의 CoreDNS를 한 장에 배치
- 시험 연결 포인트:
  - `kubectl get pods -o wide`로 IP와 Node를 같이 보게 한다.
  - "Pod IP를 직접 찍어 보는가, Service 이름으로 붙는가"를 구분시킨다.

### 4. Service 핵심 `15분`

- 학습 목표: Service를 "Pod 묶음 앞의 고정 주소"로 이해하고, selector와 endpoint를 연결 지을 수 있게 한다.
- 설명 순서:
  - 왜 Pod IP만 믿으면 안 되는가
  - Service가 selector로 Pod를 고른다
  - 선택된 결과가 endpoint로 보인다
  - 사용자는 Service IP 또는 Service 이름으로 붙는다
- 꼭 다룰 타입:
  - `ClusterIP`: 클러스터 내부 기본 타입
  - `NodePort`: 노드 포트를 통해 외부에서 접근
  - `LoadBalancer`: 클라우드 환경에서 외부 로드밸런서 연결
  - `headless Service`: 가상 IP 없이 Pod 레코드를 직접 드러내는 형태
- 강사용 핵심 문장:
  - Service가 안 되는 문제의 절반은 selector가 틀렸거나, 뒤 Pod가 Ready가 아니거나, targetPort가 안 맞는 문제다.
- 데모 시나리오:
  - `nginx` Pod 2개와 Service 1개 생성
  - selector를 일부러 틀리게 두고 `kubectl get svc`, `kubectl get endpoints` 확인
  - selector 수정 후 endpoint가 생기는 흐름 확인
- 실전 명령어:
  - `kubectl get svc`
  - `kubectl get endpoints`
  - `kubectl describe svc <name>`
  - `kubectl get pods -l app=<label> -o wide`
- 수강생에게 남길 판단 기준:
  - Service가 있는데 endpoint가 없으면 대개 selector 또는 Pod 상태 문제
  - endpoint는 있는데 응답이 이상하면 targetPort, containerPort, 앱 상태 문제

### 5. DNS 핵심 `10분`

- 학습 목표: Service 이름이 실제로 어떻게 해석되는지, namespace에 따라 왜 결과가 달라지는지 이해시킨다.
- 설명할 내용:
  - CoreDNS는 클러스터 DNS 서버다.
  - 같은 namespace에서는 짧은 이름으로도 접근 가능하다.
  - 다른 namespace에 붙을 때는 `service.namespace.svc.cluster.local` 형태를 본다.
  - headless Service는 여러 Pod IP가 응답될 수 있다.
- 데모 시나리오:
  - `dnsutils` 또는 `busybox` Pod 안에서 `nslookup` 실행
  - 같은 namespace 서비스 이름 조회
  - 다른 namespace 서비스는 FQDN으로 조회
  - headless Service와 일반 Service 결과 비교
- 강사용 핵심 문장:
  - DNS 문제인지 확인할 때는 "이름이 IP로 바뀌는가"를 먼저 본다. 바뀌었는데 접속이 안 되면 DNS가 아니라 그 다음 문제다.
- 실전 명령어:
  - `kubectl exec -it <pod> -- nslookup <service>`
  - `kubectl exec -it <pod> -- dig <service>`
  - `kubectl get pods -n kube-system`
  - `kubectl get svc -n kube-system`

### 6. Ingress 최소 이해 `10분`

- 학습 목표: Ingress를 외부 요청의 진입 규칙으로 이해하고, Service와의 관계를 끊어 읽을 수 있게 한다.
- 설명할 내용:
  - Ingress는 host/path 기반 라우팅 규칙이다.
  - Ingress는 직접 Pod를 바라보지 않고 보통 Service를 바라본다.
  - Ingress 리소스만 만들어서는 안 되고, Ingress Controller가 있어야 실제로 동작한다.
- 시험 관점에서 꼭 남길 것:
  - 문제에서 이미 Controller가 준비된 경우가 많다.
  - YAML에서 `host`, `path`, `backend service name`, `service port`를 정확히 보는 것이 중요하다.
  - Ingress가 안 되면 Ingress만 보지 말고 뒤의 Service와 Pod를 순서대로 확인한다.
- YAML 읽기 포인트:
  - `spec.rules.host`
  - `spec.rules.http.paths.path`
  - `backend.service.name`
  - `backend.service.port.number`
- 강사용 한 줄 정리:
  - Ingress는 "밖에서 들어오는 길", Service는 "클러스터 안 고정 진입점", Pod는 "실제 앱"

### 7. NetworkPolicy 최소 이해 `13분`

- 학습 목표: 허용 정책이 어떻게 통신 가능 여부를 바꾸는지 시험 수준으로 해석하게 한다.
- 먼저 바로잡을 오해:
  - NetworkPolicy는 클러스터 전체 방화벽이 아니다.
  - policy가 없는 Pod는 기본 허용처럼 보일 수 있다.
  - policy가 적용되기 시작한 Pod에 대해 ingress/egress 허용 규칙을 붙이는 구조다.
- 설명할 내용:
  - `podSelector`: 누구에게 정책을 적용할 것인가
  - `policyTypes`: ingress, egress
  - `from`, `to`: 어떤 출발지/목적지를 허용할 것인가
  - `namespaceSelector`, `podSelector`, `ipBlock`의 최소 구분
- 데모 시나리오:
  - `frontend`, `backend`, `client` Pod 준비
  - 아무 policy 없을 때는 통신 가능
  - backend를 선택하는 ingress policy 추가 후 frontend만 허용
  - client Pod에서 실패, frontend Pod에서 성공 확인
- 강사용 핵심 문장:
  - NetworkPolicy 문제는 "누가 막혔는가"보다 "정책이 어느 Pod에 붙었는가"를 먼저 봐야 한다.
- 실전 명령어:
  - `kubectl get networkpolicy`
  - `kubectl describe networkpolicy <name>`
  - `kubectl get pods --show-labels`
  - `kubectl exec -it <pod> -- wget -qO- <service>`

### 8. 트러블슈팅 드릴 `11분`

- 학습 목표: 문제를 읽고 즉시 확인 순서를 떠올리게 한다.
- 드릴 1. Pod to Pod 실패
  - 확인 순서: Pod Running 여부 -> Pod IP 확인 -> 다른 Pod에서 직접 IP로 접속 -> label, readiness, 앱 포트 확인
- 드릴 2. Pod to Service 실패
  - 확인 순서: Service 존재 -> selector -> endpoints -> targetPort -> 뒤 Pod 상태
- 드릴 3. DNS 이름 해석 실패
  - 확인 순서: `nslookup` 결과 -> CoreDNS Pod 상태 -> namespace/FQDN 오타 -> Service 존재 여부
- 드릴 4. NetworkPolicy 적용 후 통신 실패
  - 확인 순서: 어떤 Pod에 policy가 붙는지 -> ingress인지 egress인지 -> label 매칭 -> namespace 매칭
- 강사용 문제 제시 방식:
  - 문제 문장을 짧게 던지고, 수강생에게 "첫 명령 2개"만 말하게 한다.
  - 정답은 YAML 전체가 아니라 진단 순서로 채점한다.

### 9. 마무리 `4분`

- 시험 직전 암기 문장:
  - Pod는 변한다. Service는 고정 진입점이다.
  - Service 문제는 selector와 endpoints부터 본다.
  - DNS 문제는 이름이 IP로 바뀌는지부터 본다.
  - Ingress 문제도 결국 뒤의 Service와 Pod를 함께 봐야 한다.
  - NetworkPolicy는 "선택된 Pod에 허용 규칙을 붙이는 방식"으로 읽는다.
- 마무리 질문:
  - "Service는 살아 있는데 접속이 안 된다면 첫 세 가지 확인은 무엇인가"
  - "FQDN이 필요한 순간은 언제인가"

## 강사용 데모 준비물

- namespace 3개: `default`, `app`, `ops`
- Pod 세트:
  - `nginx` Pod 2개
  - `busybox` 또는 `dnsutils` Pod 1개
  - `frontend`, `backend`, `client` Pod 각 1개
- Service 세트:
  - 정상 `ClusterIP` Service 1개
  - selector가 일부러 틀린 Service 1개
  - headless Service 1개
- 정책 세트:
  - `backend`에 ingress 허용 정책 1개
- 있으면 좋은 준비:
  - Ingress 예제 YAML 1개
  - `curlimages/curl` 또는 `busybox` 이미지

## 추천 슬라이드 흐름

1. 오늘 강의의 약속
2. 시험에서 네트워크를 보는 방식
3. IP / Port / DNS 10분 압축
4. 쿠버네티스 네트워크 3원칙
5. Pod, Service, DNS 한 장 그림
6. Service 타입 비교
7. selector와 endpoints
8. Service 장애 진단 순서
9. CoreDNS와 이름 해석
10. namespace와 FQDN
11. Ingress 최소 이해
12. NetworkPolicy 읽는 법
13. 허용/차단 흐름 예시
14. 실전 트러블슈팅 4패턴
15. 시험 직전 체크리스트

## 시험용 명령어 치트시트

- `kubectl get pods -o wide`
- `kubectl get svc`
- `kubectl get endpoints`
- `kubectl describe svc <name>`
- `kubectl get networkpolicy`
- `kubectl describe networkpolicy <name>`
- `kubectl exec -it <pod> -- nslookup <service>`
- `kubectl exec -it <pod> -- dig <service>`
- `kubectl exec -it <pod> -- curl http://<service>:<port>`
- `kubectl exec -it <pod> -- wget -qO- http://<service>:<port>`

## 강의 후 과제

- 과제 1: `Service`와 `headless Service`의 DNS 조회 결과 차이를 직접 캡처해 정리한다.
- 과제 2: selector가 잘못된 Service, DNS 오타, NetworkPolicy 차단 문제를 각각 1개씩 풀어 본다.
- 과제 3: 아래 문장을 빈칸 없이 말할 수 있을 때까지 반복한다.
  - Pod 통신 문제는 Pod 상태와 직접 IP 접속부터 본다.
  - Service 문제는 selector와 endpoints부터 본다.
  - DNS 문제는 이름이 IP로 바뀌는지부터 본다.
  - Ingress 문제는 뒤의 Service와 Pod까지 함께 본다.

## 한 줄 결론

- 이 강의의 목적은 네트워크를 깊게 아는 것이 아니라, CKA에서 네트워크 문제를 만났을 때 어디서 끊기는지 가장 짧은 순서로 줄여 가는 습관을 만드는 것이다.
