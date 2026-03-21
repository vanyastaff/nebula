import { useState } from "react";
import { useNavigate, useParams } from "react-router";
import type { CredentialFormData } from "./domain/types";
import { useCredentialStore } from "./store";
import { CredentialForm } from "./ui/CredentialForm";

export function CredentialFormPage() {
  const { id } = useParams<{ id?: string }>();
  const navigate = useNavigate();
  const store = useCredentialStore();
  const [isSubmitting, setIsSubmitting] = useState(false);

  async function handleSubmit(data: CredentialFormData) {
    setIsSubmitting(true);
    try {
      await store.create({
        name: data.name,
        kind: data.kind,
        state: JSON.stringify(data.state),
        tags: null,
      });
      void navigate("/credentials");
    } catch (err) {
      console.error("Failed to save credential:", err);
    } finally {
      setIsSubmitting(false);
    }
  }

  return (
    <CredentialForm
      onSubmit={(data) => void handleSubmit(data)}
      onCancel={() => void navigate(id ? `/credentials/${id}` : "/credentials")}
      isSubmitting={isSubmitting}
    />
  );
}
