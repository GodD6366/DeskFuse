package winservice

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// workflowList returns the service's revisioned workflow catalog in the shape
// the settings page consumes: {definitions, catalog}. The service exposes no
// run list for the synchronous RunWorkflow command, so `runs` stays absent and
// the page shows no fabricated history.
func (s *Service) workflowList(ctx context.Context, result any) error {
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	definitionsRaw, err := s.callVariant(ctx, "GetWorkflows", "Workflows")
	if err != nil {
		return err
	}
	var definitions struct {
		Workflows []json.RawMessage `json:"workflows"`
	}
	if err := json.Unmarshal(definitionsRaw, &definitions); err != nil {
		return fmt.Errorf("工作流目录无效: %w", err)
	}
	catalogRaw, err := s.callVariant(ctx, "GetWorkflowCatalog", "WorkflowCatalog")
	if err != nil {
		return err
	}
	var catalog struct {
		JSON string `json:"json"`
	}
	if err := json.Unmarshal(catalogRaw, &catalog); err != nil {
		return fmt.Errorf("工作流同步状态无效: %w", err)
	}
	if catalog.JSON == "" {
		return errors.New("服务返回了空的工作流同步状态")
	}
	var catalogValue json.RawMessage
	if err := json.Unmarshal([]byte(catalog.JSON), &catalogValue); err != nil {
		return fmt.Errorf("工作流同步状态不是有效 JSON: %w", err)
	}
	return assign(result, map[string]any{
		"outcome": "applied",
		"value": map[string]any{
			"definitions": nonNilRaw(definitions.Workflows),
			"catalog":     catalogValue,
		},
	})
}

// workflowSave persists one deskfuse_core::workflow::Workflow. The page model
// already uses the core field names; the service validates the result.
func (s *Service) workflowSave(ctx context.Context, args map[string]any, result any) error {
	workflow, ok := args["workflow"]
	if !ok || workflow == nil {
		return invalidParams("缺少 workflow")
	}
	switch workflow.(type) {
	case map[string]any, json.RawMessage:
	default:
		return invalidParams("workflow 必须是对象")
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callCommand(ctx, tagged("SaveWorkflow", map[string]any{"workflow": workflow}), "workflow.save", result)
}

// workflowDelete mirrors the macOS confirmation requirement and removes the
// definition from the service catalog.
func (s *Service) workflowDelete(ctx context.Context, args map[string]any, result any) error {
	confirmed, _ := boolValue(args, "confirm")
	if !confirmed {
		return &RemoteError{Code: "confirmation_required", Message: "删除工作流需要 confirm:true"}
	}
	id, err := requiredString(args, []string{"id"}, "缺少工作流 ID")
	if err != nil {
		return err
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callCommand(ctx, tagged("DeleteWorkflow", map[string]any{"id": id}), "workflow.delete", result)
}

func (s *Service) workflowRestoreConflict(ctx context.Context, args map[string]any, result any) error {
	id, err := requiredString(args, []string{"id"}, "缺少冲突 ID")
	if err != nil {
		return err
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callCommand(ctx, tagged("RestoreWorkflowConflict", map[string]any{"conflict_id": id}), "workflow.restore_conflict", result)
}

// workflowCancel forwards the command so the service's real rejection reaches
// the page. Windows workflows run synchronously in a bounded request and the
// service currently has no cancellation path.
func (s *Service) workflowCancel(ctx context.Context, args map[string]any, result any) error {
	runID, err := requiredString(args, []string{"run_id"}, "缺少任务 ID")
	if err != nil {
		return err
	}
	if err := s.ensureSession(ctx); err != nil {
		return err
	}
	return s.callCommand(ctx, tagged("CancelWorkflow", map[string]any{"run_id": runID}), "workflow.cancel", result)
}
