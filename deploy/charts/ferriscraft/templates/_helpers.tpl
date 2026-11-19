{{- define "ferriscraft.fullname" -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "ferriscraft.labels" -}}
app.kubernetes.io/name: ferriscraft
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "ferriscraft.image" -}}
{{- $tag := .tag | default .global.imageTag -}}
{{- printf "%s/%s:%s" .global.imageRegistry .repository $tag -}}
{{- end -}}

{{- define "ferriscraft.forwardingSecretName" -}}
{{- if .Values.secrets.existingForwardingSecret -}}
{{- .Values.secrets.existingForwardingSecret -}}
{{- else -}}
{{- printf "%s-forwarding" (include "ferriscraft.fullname" .) -}}
{{- end -}}
{{- end -}}

{{- define "ferriscraft.orchestratorTokenSecretName" -}}
{{- if .Values.secrets.existingOrchestratorTokenSecret -}}
{{- .Values.secrets.existingOrchestratorTokenSecret -}}
{{- else if .Values.secrets.devOrchestratorToken -}}
{{- printf "%s-orch-token" (include "ferriscraft.fullname" .) -}}
{{- end -}}
{{- end -}}
