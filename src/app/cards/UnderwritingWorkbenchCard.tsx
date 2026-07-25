import {
  CrmContext,
  EmptyState,
  ExtensionPointApiActions,
  Link,
  Text,
} from '@hubspot/ui-extensions';
import { hubspot } from '@hubspot/ui-extensions';

interface CrmExtensionProps {
  context: CrmContext;
  actions: ExtensionPointApiActions<'crm.record.tab'>;
}

hubspot.extend<'crm.record.tab'>(({ context, actions }: CrmExtensionProps) => (
  <UnderwritingWorkbenchCard context={context} actions={actions} />
));

const UnderwritingWorkbenchCard = ({ context }: CrmExtensionProps) => {
  const dealId = context.crm.objectId;
  const portalId = context.portal.id;
  const query = `hubspotPortalId=${encodeURIComponent(portalId)}&hubspotDealId=${encodeURIComponent(dealId)}`;
  const workbenchUrl = `https://uw.sagesure.io/jobs?${query}`;
  const integratedUrl = `https://app.sagesure.io/underwriting?${query}`;

  return (
    <EmptyState
      title="SageSure Underwriting Workbench"
      layout="vertical"
      imageName="documents"
    >
      <Text>
        Open this HubSpot deal in the linked underwriting workflow while
        preserving the development-portal and deal correlation identifiers.
      </Text>
      <Link href={workbenchUrl}>Open standalone UW Workbench</Link>
      <Link href={integratedUrl}>Open underwriting in SageSure App</Link>
    </EmptyState>
  );
};
